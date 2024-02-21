use std::ffi::OsString;
use std::num::IntErrorKind;
use std::ops::Range;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};

use nom::error::FromExternalError;
use nom::sequence::terminated;
use nom::{
    bytes::complete::tag,
    character::complete::one_of,
    combinator::opt,
    multi::many1,
    IResult,
};
use nom::{AsChar, FindToken, Finish};
use phf::phf_map;

use crate::config_file::{CleanupAge, FileOwner, Line, LineAction, LineType, Mode, Spanned};

// Saturating_mul here because const trait isn't stable at time of writing
static NANOSECOND: Duration = Duration::from_nanos(1);
static MICROSECOND: Duration = NANOSECOND.saturating_mul(1000);
static MILLISECOND: Duration = MICROSECOND.saturating_mul(1000);
static SECOND: Duration = MILLISECOND.saturating_mul(1000);
static MINUTE: Duration = SECOND.saturating_mul(60);
static HOUR: Duration = MINUTE.saturating_mul(60);
static DAY: Duration = HOUR.saturating_mul(24);
static WEEK: Duration = DAY.saturating_mul(7);

// This is how systemd defines them, a bit weird but okay
static MONTH: Duration = DAY
    .saturating_mul(30)
    .saturating_add(HOUR.saturating_mul(10))
    .saturating_add(MINUTE.saturating_mul(30));
static YEAR: Duration = DAY
    .saturating_mul(365)
    .saturating_add(HOUR.saturating_mul(6));

static DURATION_KEYWORDS: phf::Map<&'static [u8], Duration> = phf_map! {
    b"seconds" => SECOND,
    b"second" => SECOND,
    b"sec" => SECOND,
    b"s" => SECOND,
    b"minutes" => MINUTE,
    b"minute" => MINUTE,
    b"min" => MINUTE,
    b"months" => MONTH,
    b"month" => MONTH,
    b"M" => MONTH,
    b"msec" => MILLISECOND,
    b"ms" => MILLISECOND,
    b"m" => MINUTE,
    b"hours" => HOUR,
    b"hour" => HOUR,
    b"hr" => HOUR,
    b"h" => HOUR,
    b"days" => DAY,
    b"day" => DAY,
    b"d" => DAY,
    b"weeks" => WEEK,
    b"week" => WEEK,
    b"w" => WEEK,
    b"years" => YEAR,
    b"year" => YEAR,
    b"y" => YEAR,
    b"usec" => MICROSECOND,
    b"us" => MICROSECOND,
    b"\xcd\xbcs" => MICROSECOND, // μs U+03bc GREEK LETTER MU
    b"\xc2\xb5s" => MICROSECOND, // µs U+b5 MICRO SIGN
    b"nsec" => NANOSECOND,
    b"ns" => NANOSECOND,
    b"" => NANOSECOND,
};

fn parse_duration_part(input: &[u8]) -> IResult<&[u8], Duration> {
    let split_idx = input
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(input.len());
    let (count, input2) = input.split_at(split_idx);
    let count = u64::from_str(std::str::from_utf8(count).unwrap()).map_err(|e| {
        nom::Err::Error(nom::error::Error::from_external_error(
            input,
            nom::error::ErrorKind::MapRes,
            e,
        ))
    })?;
    let input = input2;
    let split_idx = input
        .iter()
        .position(|c| c.is_ascii() && !c.is_ascii_alphabetic())
        .unwrap_or(input.len());
    let (key, input2) = input.split_at(split_idx);
    let unit = DURATION_KEYWORDS.get(key).ok_or_else(|| {
        nom::Err::Error(nom::error::ParseError::from_error_kind(
            input,
            nom::error::ErrorKind::MapOpt,
        ))
    })?;
    let input = input2;

    // saturating_mul is only for u32, so do it ourselves
    let total_nanos = unit.subsec_nanos() as u128 * count as u128;
    let nanos = (total_nanos % 1_000_000_000) as u32;
    let extra_secs = (total_nanos / 1_000_000_000) as u64;
    let secs = unit
        .as_secs()
        .saturating_mul(count)
        .saturating_add(extra_secs);

    Ok((input, Duration::new(secs, nanos)))
}

fn parse_duration(input: &[u8]) -> IResult<&[u8], Duration> {
    let (mut input, mut acc) = parse_duration_part(input)?;
    while !input.is_empty() {
        let (i, o) = parse_duration_part(input)?;
        acc = acc.saturating_add(o);
        input = i;
    }
    return Ok((b"".as_slice(), acc));
}

fn parse_cleanup_age_by(input: &[u8]) -> IResult<&[u8], CleanupAge> {
    let (input, second_level_flag) = opt(tag(b"~"))(input)?;
    let (input, chars) = many1(one_of(b"aAbBcCmM".as_slice()))(input)?;

    let mut flags = CleanupAge {
        second_level: second_level_flag.is_some(),
        ..Default::default()
    };
    for ch in chars {
        match ch {
            'a' => flags.consider_atime = true,
            'A' => flags.consider_atime_dir = true,
            'b' => flags.consider_btime = true,
            'B' => flags.consider_btime_dir = true,
            'c' => flags.consider_ctime = true,
            'C' => flags.consider_ctime_dir = true,
            'm' => flags.consider_mtime = true,
            'M' => flags.consider_mtime_dir = true,
            _ => unreachable!(),
        }
    }

    Ok((input, flags))
}

fn try_optional<'a, B: AsRef<[u8]> + 'a, T: 'a, E: 'a, F: FnOnce(B) -> Result<T, E> + 'a>(
    f: F,
) -> impl FnOnce(B) -> Result<Option<T>, E> + 'a {
    |f2| optional(f)(f2).transpose()
}

fn optional<B: AsRef<[u8]>, T, F: FnOnce(B) -> T>(f: F) -> impl FnOnce(B) -> Option<T> {
    |input| {
        if input.as_ref() == b"-" {
            None
        } else {
            Some(f(input))
        }
    }
}

fn parse_cleanup_age(input: &[u8]) -> Result<CleanupAge, nom::error::Error<&[u8]>> {
    let (input, mut cleanup_age) = terminated(parse_cleanup_age_by, tag(b":"))(input).finish()?;
    let (&[], duration) = parse_duration(input).finish()? else {
        // parse_duration didn't consume the whole input
        todo!()
    };

    cleanup_age.age = duration;

    Ok(cleanup_age)
}

#[allow(unused)]
fn parse_line(mut input: FileSpan) -> Result<Line, ()> {
    let line_type = take_field(&mut input)?.as_deref().try_map(parse_type)?;
    take_inline_whitespace(&mut input)?;
    let path = take_field(&mut input)?.map(|field| {
        let vec = field.to_vec();
        let os_string = OsString::from_vec(vec);
        PathBuf::from(os_string)
    });
    take_inline_whitespace(&mut input)?;
    let mode = take_field(&mut input)?
        .as_deref()
        .try_map(try_optional(parse_mode))?;
    take_inline_whitespace(&mut input)?;
    let owner = take_field(&mut input)?.try_map(try_optional(parse_user))?;
    take_inline_whitespace(&mut input)?;
    let group = take_field(&mut input)?.try_map(try_optional(parse_user))?;
    take_inline_whitespace(&mut input)?;
    let take_field = take_field(&mut input)?;
    let age = take_field
        .as_deref()
        .try_map(try_optional(parse_cleanup_age));
    let Ok(age) = age else { todo!() };
    let age = age.map(|age| age.unwrap_or(CleanupAge::EMPTY));
    take_inline_whitespace(&mut input)?;
    let remaining = Spanned::new(input.bytes, input.file, input.char_range);
    let argument = remaining.map(optional(|bytes: &[u8]| OsString::from_vec(bytes.to_vec())));

    Ok(Line {
        line_type,
        path,
        mode,
        owner,
        group,
        age,
        argument,
    })
}

pub struct FileSpan<'a> {
    bytes: &'a [u8],
    file: &'a Path,
    char_range: Range<usize>,
    cursor: usize,
}

impl<'a> FileSpan<'a> {
    fn peek(&self) -> Option<&u8> {
        self.bytes.get(self.cursor)
    }
    fn advance(&mut self) {
        self.cursor += 1;
    }
    fn next(&mut self) -> Option<u8> {
        let byte = *self.peek()?;
        self.advance();
        Some(byte)
    }
    fn split(&mut self) -> Self {
        let split = Self {
            bytes: &self.bytes[..self.cursor],
            file: self.file,
            char_range: self.char_range.start..(self.char_range.start + self.cursor),
            cursor: 0,
        };
        *self = Self {
            bytes: &self.bytes[self.cursor..],
            file: self.file,
            char_range: (self.char_range.start + self.cursor)..self.char_range.end,
            cursor: 0,
        };
        split
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[self.cursor..]
    }
    #[cfg(test)]
    fn from_slice(bytes: &'a [u8], file: &'a Path) -> Self {
        Self {
            bytes,
            file,
            char_range: 0..bytes.len(),
            cursor: 0,
        }
    }
}

fn take_inline_whitespace(input: &mut FileSpan<'_>) -> Result<(), ()> {
    match input.peek() {
        Some(b' ' | b'\t') => input.advance(),
        None => todo!(),    // Empty input
        Some(_) => todo!(), // There was zero whitespace
    }
    loop {
        match input.peek() {
            Some(b' ' | b'\t') => input.advance(),
            None => todo!(),
            Some(_) => {
                input.split();
                return Ok(());
            }
        }
    }
}

fn take_field(input: &mut FileSpan<'_>) -> Result<Spanned<Box<[u8]>>, ()> {
    let Some(&first_char) = input.peek() else {
        // Unexpected end of line
        todo!()
    };
    let mut field = Vec::new();
    let quotation = if matches!(first_char, b'\'' | b'"') {
        // We have a quoted string
        input.next()
    } else {
        None
        // Unquoted field
    };
    loop {
        let Some(&c) = input.peek() else {
            // End of line
            todo!()
        };
        match c {
            b'\'' | b'"' if Some(c) == quotation => {
                input.advance();
                break;
            }
            b' ' | b'\t' if quotation.is_none() => break,
            b'\\' => {
                input.advance();
                let Some(character) = input.next() else {
                    // End of line parsing escape
                    todo!("End of input parsing escape sequence")
                };
                match character {
                    b'x' => {
                        // Hexadecimal: \xhh
                        let Some(digits) = input.as_bytes().get(..2) else {
                            todo!("End of input parsing hex escape")
                        };
                        let byte = match std::str::from_utf8(digits)
                            .map_err(|_| IntErrorKind::InvalidDigit)
                            .and_then(|s| u8::from_str_radix(s, 16).map_err(|e| e.kind().clone()))
                        {
                            Ok(byte) => byte,
                            Err(IntErrorKind::InvalidDigit) => {
                                todo!("Invalid hex sequence parsing hex escape")
                            }
                            _ => todo!(),
                        };
                        field.push(byte);
                    }
                    b'0'..=b'7' => {
                        // Octal: \OOO
                        todo!("Octal escape sequences are not supported")
                    }
                    b'n' => field.push(b'\n'),
                    b'r' => field.push(b'\r'),
                    b't' => field.push(b'\t'),
                    b'\'' | b'"' | b'\\' => field.push(c),
                    _ => todo!("Unrecognized escape sequence"),
                }
            }
            _ => {
                input.advance();
                field.push(c);
            }
        }
    }
    Ok(Spanned::new(
        field.into_boxed_slice(),
        input.file,
        input.split().char_range,
    ))
}

fn parse_mode(mut input: &[u8]) -> Result<Mode, ()> {
    let (masked, keep_existing) = match input.first() {
        Some(b':') => {
            input = &input[1..];
            (false, true)
        }
        Some(b'~') => {
            input = &input[1..];
            (true, false)
        }
        _ => (false, false),
    };
    let Ok(string) = std::str::from_utf8(input) else {
        todo!() // Invalid utf8
    };
    let Ok(mode) = u32::from_str_radix(string, 8) else {
        todo!()
    };
    Ok(Mode {
        value: mode,
        masked,
        keep_existing,
    })
}
fn parse_user(input: Box<[u8]>) -> Result<FileOwner, ()> {
    let vec = input.into_vec();
    let id = std::str::from_utf8(&vec)
        .ok()
        .and_then(|s| u32::from_str(s).ok());
    Ok(match id {
        Some(id) => FileOwner::Id(id),
        None => FileOwner::Name(OsString::from_vec(vec)),
    })
}

fn parse_type(input: &[u8]) -> Result<LineType, ()> {
    let Some(char) = input.first() else { todo!() };
    let char = char.as_char();
    let Some(modifiers) = input.get(1..) else {
        todo!()
    };
    let plus = modifiers.find_token('+');
    let minus = modifiers.find_token('-');
    let exclamation = modifiers.find_token('!');
    let equals = modifiers.find_token('=');
    let tilde = modifiers.find_token('~');
    let caret = modifiers.find_token('^');
    let action = match char {
        'f' => LineAction::CreateFile,
        'w' => LineAction::WriteFile,
        'd' | 'v' | 'q' | 'Q' => LineAction::CreateAndCleanUpDirectory,
        'D' => LineAction::CreateAndRemoveDirectory,
        'e' => LineAction::CleanUpDirectory,
        'p' => LineAction::CreateFifo,
        'L' => LineAction::CreateSymlink,
        'c' => LineAction::CreateCharDevice,
        'b' => LineAction::CreateBlockDevice,
        'C' => LineAction::Copy,
        'x' => LineAction::Ignore,
        'X' => LineAction::IgnoreNonRecursive,
        'r' => LineAction::Remove,
        'R' => LineAction::RemoveRecursive,
        'z' => LineAction::SetMode,
        'Z' => LineAction::SetModeRecursive,
        't' => LineAction::SetXattr,
        'T' => LineAction::SetXattrRecursive,
        'h' => LineAction::SetAttr,
        'H' => LineAction::SetAttrRecursive,
        'a' => LineAction::SetAcl,
        'A' => LineAction::SetAclRecursive,
        _ => todo!(),
    };
    let recreate = if plus {
        if matches!(char, 'f' | 'w' | 'p' | 'L' | 'c' | 'b' | 'C' | 'a' | 'A') {
            true
        } else {
            // error
            todo!()
        }
    } else {
        false
    };
    let boot = exclamation;
    let noerror = minus;
    let force = equals;
    if tilde || caret {
        todo!()
    }
    Ok(LineType {
        action,
        recreate,
        boot,
        noerror,
        force,
    })
}

#[cfg(test)]
mod test {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
        str::FromStr,
    };

    use crate::{
        config_file::{CleanupAge, Line, LineAction, LineType, Spanned},
        parser::{
            parse_duration, parse_duration_part, parse_line, FileSpan, MICROSECOND, NANOSECOND,
            SECOND, WEEK,
        },
    };

    #[test]
    fn test_duration_part() {
        assert_eq!(parse_duration_part(b"1s"), Ok((b"".as_slice(), SECOND)));
        assert_eq!(
            parse_duration_part("1µs".as_bytes()),
            Ok((b"".as_slice(), MICROSECOND))
        );
        assert_eq!(
            parse_duration_part("123456789".as_bytes()),
            Ok((b"".as_slice(), NANOSECOND * 123_456_789))
        );
    }

    #[test]
    fn test_duration() {
        assert_eq!(parse_duration(b"1s1m"), Ok((b"".as_slice(), SECOND * 61)));
        assert_eq!(
            parse_duration("6days23hr59m59sec999ms999µs1000ns".as_bytes()),
            Ok((b"".as_slice(), WEEK))
        );
    }

    #[test]
    fn test_line() {
        let dummy_file = Path::new("");
        assert_eq!(
            parse_line(FileSpan::from_slice(b"L+ /run/gdm/.config/pulse/default.pa - - - - /nix/store/whibfps24g91fx9i63m2wdyl87dfadnn-default.pa", dummy_file)),
            Ok(Line {
                line_type: Spanned::new(LineType { action: LineAction::CreateSymlink, recreate: true, boot: false, noerror: false, force: false }, dummy_file, 0..2 ),
                path: Spanned::new(PathBuf::from_str("/run/gdm/.config/pulse/default.pa").unwrap(), dummy_file, 3..36),
                mode: Spanned::new(None, dummy_file, 37..38),
                owner: Spanned::new(None, dummy_file, 39..40),
                group: Spanned::new(None, dummy_file, 41..42),
                age: Spanned::new(CleanupAge::EMPTY, dummy_file, 43..44),
                argument: Spanned::new(Some(OsString::from("/nix/store/whibfps24g91fx9i63m2wdyl87dfadnn-default.pa")), dummy_file, 45..99)
            })
        );
    }
}
