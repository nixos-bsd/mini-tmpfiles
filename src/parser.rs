use std::ffi::OsString;
use std::num::{IntErrorKind, ParseIntError};
use std::ops::Range;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};

use phf::phf_map;

use crate::config_file::{
    CleanupAge, FileOwner, Line, LineAction, LineType, Mode, ModeBehavior, Spanned,
};

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
    b"nsec" => NANOSECOND,
    b"ns" => NANOSECOND,
    b"usec" => MICROSECOND,
    b"us" => MICROSECOND,
    b"\xcd\xbcs" => MICROSECOND, // μs U+03bc GREEK LETTER MU
    b"\xc2\xb5s" => MICROSECOND, // µs U+b5 MICRO SIGN
    b"msec" => MILLISECOND,
    b"ms" => MILLISECOND,
    b"seconds" => SECOND,
    b"second" => SECOND,
    b"sec" => SECOND,
    b"s" => SECOND,
    b"" => SECOND,
    b"minutes" => MINUTE,
    b"minute" => MINUTE,
    b"min" => MINUTE,
    b"m" => MINUTE,
    b"hours" => HOUR,
    b"hour" => HOUR,
    b"hr" => HOUR,
    b"h" => HOUR,
    b"months" => MONTH,
    b"month" => MONTH,
    b"M" => MONTH,
    b"days" => DAY,
    b"day" => DAY,
    b"d" => DAY,
    b"weeks" => WEEK,
    b"week" => WEEK,
    b"w" => WEEK,
    b"years" => YEAR,
    b"year" => YEAR,
    b"y" => YEAR,
};

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    EndOfLine,
    IllegalParseType(u8),
    LeadingWhitespace,
    EmptyParseType,
    InvalidTypeCombination(u8, u8),
    InvalidTypeModifier(u8),
    InvalidMode,
    DuplicateTypeModifier(u8),
    IDKWhatAServiceCredentialIs,
    InvalidCleanupAge(CleanupParseError),
    InvalidUsername,
    NullInPath,
    Field(FieldParseError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum FieldParseError {
    UnrecognizedEscape(u8),
    TrailingBackslash,
    UnfinishedHexEscape,
    UnsupportedOctalEscape,
    QuoteInUnquotedField,
    InvalidHexEscape,
    JunkAfterQuotes,
    EndOfLine,
}

impl From<CleanupParseError> for ParseError {
    fn from(value: CleanupParseError) -> Self {
        Self::InvalidCleanupAge(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CleanupParseError {
    InvalidDurationInt(ParseIntError),
    InvalidDurationKeyword(Box<[u8]>),
    DuplicateCleanupSpecifier(u8),
    InvalidCleanupSpecifier(u8),
    Malformed(Box<[u8]>),
    OverflowedDuration(Box<[u8]>),
    EmptyCleanupSpecifierList,
}

impl From<FieldParseError> for ParseError {
    fn from(value: FieldParseError) -> Self {
        Self::Field(value)
    }
}

fn take_from_slice_while<'a>(
    slice: &mut &'a [u8],
    mut predicate: impl FnMut(&'a u8) -> bool,
) -> &'a [u8] {
    let split_idx = slice
        .iter()
        .position(|b| !predicate(b))
        .unwrap_or(slice.len());
    let (taken, remaining) = slice.split_at(split_idx);
    *slice = remaining;
    taken
}

fn take_string_from_slice<'a>(slice: &mut &'a [u8], s: &str) -> Option<&'a [u8]> {
    let remaining = slice.strip_prefix(s.as_bytes())?;
    let taken = &slice[..s.as_bytes().len()];
    *slice = remaining;
    Some(taken)
}

fn parse_duration_part(input: &mut &[u8]) -> Result<Duration, CleanupParseError> {
    let original_input = *input;
    let count = take_from_slice_while(input, u8::is_ascii_digit);
    let count = u64::from_str(std::str::from_utf8(count).unwrap())
        .map_err(CleanupParseError::InvalidDurationInt)?;
    let key = take_from_slice_while(input, |c| c.is_ascii_alphabetic() || !c.is_ascii());
    let unit = DURATION_KEYWORDS
        .get(key)
        .ok_or_else(|| CleanupParseError::InvalidDurationKeyword(key.into()))?;

    // saturating_mul is only for u32, so do it ourselves
    let total_nanos = unit.subsec_nanos() as u128 * count as u128;
    let nanos = (total_nanos % 1_000_000_000) as u32;
    let extra_secs = (total_nanos / 1_000_000_000) as u64;
    let secs = unit
        .as_secs()
        .checked_mul(count)
        .and_then(|secs| secs.checked_add(extra_secs))
        .ok_or_else(|| CleanupParseError::OverflowedDuration(original_input.into()))?;

    Ok(Duration::new(secs, nanos))
}

fn parse_duration(mut input: &[u8]) -> Result<Duration, CleanupParseError> {
    let original_input = input;
    let mut acc = parse_duration_part(&mut input)?;
    while !input.is_empty() {
        acc = acc
            .checked_add(parse_duration_part(&mut input)?)
            .ok_or_else(|| CleanupParseError::OverflowedDuration(original_input.into()))
            .unwrap();
    }
    Ok(acc)
}

fn parse_cleanup_age_by(mut input: &[u8]) -> Result<CleanupAge, CleanupParseError> {
    let second_level = take_string_from_slice(&mut input, "~").is_some();

    let mut flags = CleanupAge {
        second_level,
        ..Default::default()
    };
    if input.is_empty() {
        Err(CleanupParseError::EmptyCleanupSpecifierList)?
    }
    for &ch in input {
        let field = match ch.into() {
            'a' => &mut flags.consider_atime,
            'A' => &mut flags.consider_atime_dir,
            'b' => &mut flags.consider_btime,
            'B' => &mut flags.consider_btime_dir,
            'c' => &mut flags.consider_ctime,
            'C' => &mut flags.consider_ctime_dir,
            'm' => &mut flags.consider_mtime,
            'M' => &mut flags.consider_mtime_dir,
            _ => Err(CleanupParseError::InvalidCleanupSpecifier(ch))?,
        };
        if *field {
            Err(CleanupParseError::DuplicateCleanupSpecifier(ch))?
        } else {
            *field = true;
        }
    }

    Ok(flags)
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

fn parse_cleanup_age(input: &[u8]) -> Result<CleanupAge, CleanupParseError> {
    let (mut cleanup_age, duration) =
        match input.split(|&c| c == b':').collect::<Vec<_>>().as_slice() {
            [] => unreachable!(),
            &[duration] => (CleanupAge::EMPTY, duration),
            &[cleanup_age, duration] => (parse_cleanup_age_by(cleanup_age)?, duration),
            [..] => Err(CleanupParseError::Malformed(input.into()))?,
        };

    cleanup_age.age = parse_duration(duration)?;

    Ok(cleanup_age)
}

#[allow(unused)]
pub fn parse_line(mut input: FileSpan) -> Result<Line, ParseError> {
    if matches!(input.bytes.first(), Some(b' ' | b'\t')) {
        return Err(ParseError::LeadingWhitespace);
    }
    let line_type = take_field(&mut input)?.as_deref().try_map(parse_type)?;
    take_inline_whitespace(&mut input)?;
    let path = take_field(&mut input)?.try_map(|field| {
        let vec = field.to_vec();
        if vec.contains(&b'\0') {
            return Err(ParseError::NullInPath);
        }
        let os_string = OsString::from_vec(vec);
        Ok(PathBuf::from(os_string))
    })?;
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
        .try_map(try_optional(parse_cleanup_age))?;
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
}

impl<'a> FileSpan<'a> {
    #[cfg(any(test, fuzzing))]
    pub fn from_slice(bytes: &'a [u8], file: &'a Path) -> Self {
        Self {
            bytes,
            file,
            char_range: 0..bytes.len(),
        }
    }
    fn cursor(&mut self) -> SpanCursor<'_, 'a> {
        SpanCursor {
            span: self,
            cursor: 0,
        }
    }
}

struct SpanCursor<'a, 'b> {
    span: &'a mut FileSpan<'b>,
    cursor: usize,
}

impl<'a, 'b> SpanCursor<'a, 'b> {
    fn peek(&self) -> Option<&u8> {
        self.span.bytes.get(self.cursor)
    }
    fn advance(&mut self) {
        self.cursor += 1;
    }
    fn split_off_beginning(self) -> FileSpan<'b> {
        let split = FileSpan {
            bytes: &self.span.bytes[..self.cursor],
            file: self.span.file,
            char_range: self.span.char_range.start..(self.span.char_range.start + self.cursor),
        };
        *self.span = FileSpan {
            bytes: &self.span.bytes[self.cursor..],
            file: self.span.file,
            char_range: (self.span.char_range.start + self.cursor)..self.span.char_range.end,
        };
        split
    }

    fn as_bytes(&self) -> &'b [u8] {
        &self.span.bytes[self.cursor..]
    }
}

fn take_inline_whitespace(input: &mut FileSpan) -> Result<(), ParseError> {
    let mut cursor = input.cursor();
    match cursor.peek() {
        Some(b' ' | b'\t') => cursor.advance(),
        None => return Err(ParseError::EndOfLine), // Empty input
        Some(_) => todo!(),                        // There was zero whitespace
    }
    loop {
        match cursor.peek() {
            Some(b' ' | b'\t') => cursor.advance(),
            Some(_) | None => {
                cursor.split_off_beginning();
                return Ok(());
            }
        }
    }
}

fn take_field<'a>(input: &mut FileSpan<'a>) -> Result<Spanned<'a, Box<[u8]>>, FieldParseError> {
    let mut cursor = input.cursor();
    let Some(&first_char) = cursor.peek() else {
        return Err(FieldParseError::EndOfLine);
    };
    let mut field = Vec::new();
    let quotation = if matches!(first_char, b'\'' | b'"') {
        // We have a quoted string
        cursor.advance();
        Some(first_char)
    } else {
        None
        // Unquoted field
    };
    loop {
        match cursor.peek().copied() {
            Some(b'\'' | b'"') if cursor.peek().copied() == quotation => {
                cursor.advance();
                let next = cursor.peek().copied();
                if !matches!(next, Some(b' ' | b'\t') | None) {
                    Err(FieldParseError::JunkAfterQuotes)?
                }
                break;
            }
            Some(b' ' | b'\t') | None if quotation.is_none() => break,
            Some(b'\'' | b'"') if quotation.is_none() => {
                return Err(FieldParseError::QuoteInUnquotedField)
            }
            Some(b'\\') => {
                cursor.advance();
                let Some(&character) = cursor.peek() else {
                    // End of line parsing escape
                    return Err(FieldParseError::TrailingBackslash);
                };
                cursor.advance();
                match character {
                    b'x' => {
                        // Hexadecimal: \xhh
                        let Some(digits) = cursor.as_bytes().get(..2) else {
                            return Err(FieldParseError::UnfinishedHexEscape);
                        };
                        cursor.advance();
                        cursor.advance();
                        let byte = match std::str::from_utf8(digits)
                            .map_err(|_| IntErrorKind::InvalidDigit)
                            .and_then(|s| u8::from_str_radix(s, 16).map_err(|e| e.kind().clone()))
                        {
                            Ok(byte) => byte,
                            Err(IntErrorKind::InvalidDigit) => {
                                return Err(FieldParseError::InvalidHexEscape)
                            }
                            _ => todo!(),
                        };
                        field.push(byte);
                    }
                    b'0'..=b'7' => {
                        // Octal: \OOO
                        return Err(FieldParseError::UnsupportedOctalEscape);
                    }
                    b'n' => field.push(b'\n'),
                    b'r' => field.push(b'\r'),
                    b't' => field.push(b'\t'),
                    b'\'' | b'"' | b'\\' => field.push(character),
                    _ => return Err(FieldParseError::UnrecognizedEscape(character)),
                }
            }
            Some(c) => {
                cursor.advance();
                field.push(c);
            }
            None => Err(FieldParseError::EndOfLine)?,
        }
    }
    if cursor.cursor == 0 {
        todo!()
    }
    Ok(Spanned::new(
        field.into_boxed_slice(),
        cursor.span.file,
        cursor.split_off_beginning().char_range,
    ))
}

fn parse_mode(mut input: &[u8]) -> Result<Mode, ParseError> {
    let mode_behavior = match input.first() {
        Some(b':') => ModeBehavior::KeepExisting,
        Some(b'~') => ModeBehavior::Masked,
        _ => ModeBehavior::Default,
    };
    if mode_behavior != ModeBehavior::Default {
        input = &input[1..];
    }
    if !(3..=4).contains(&input.len()) {
        return Err(ParseError::InvalidMode);
    }
    let Ok(string) = std::str::from_utf8(input) else {
        return Err(ParseError::InvalidMode);
    };
    let Ok(mode) = u32::from_str_radix(string, 8) else {
        return Err(ParseError::InvalidMode);
    };
    Ok(Mode {
        value: mode,
        mode_behavior,
    })
}
fn parse_user(input: Box<[u8]>) -> Result<FileOwner, ParseError> {
    let Ok(s) = std::str::from_utf8(&input) else {
        return Err(ParseError::InvalidUsername);
    };
    Ok(if let Ok(id) = u32::from_str(s) {
        FileOwner::Id(id)
    } else {
        FileOwner::Name(s.to_owned())
    })
}

fn parse_type(input: &[u8]) -> Result<LineType, ParseError> {
    let Some(&char) = input.first() else {
        return Err(ParseError::EmptyParseType);
    };
    let Some(modifiers) = input.get(1..) else {
        todo!()
    };
    let action = match char::from(char) {
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
        _ => return Err(ParseError::IllegalParseType(char)),
    };
    let mut plus = false;
    let mut minus = false;
    let mut exclamation = false;
    let mut equals = false;
    let mut tilde = false;
    let mut caret = false;
    for &c in modifiers {
        let var = match c {
            b'+' => &mut plus,
            b'-' => &mut minus,
            b'!' => &mut exclamation,
            b'=' => &mut equals,
            b'~' => &mut tilde,
            b'^' => &mut caret,
            _ => return Err(ParseError::InvalidTypeModifier(c)),
        };
        if *var {
            return Err(ParseError::DuplicateTypeModifier(c));
        } else {
            *var = true;
        }
    }
    let recreate = if plus {
        if matches!(
            char.into(),
            'f' | 'w' | 'p' | 'L' | 'c' | 'b' | 'C' | 'a' | 'A'
        ) {
            true
        } else {
            return Err(ParseError::InvalidTypeCombination(char, b'+'));
        }
    } else {
        false
    };
    let boot = exclamation;
    let noerror = minus;
    let force = equals;
    let base64_decode = tilde;
    if caret {
        return Err(ParseError::IDKWhatAServiceCredentialIs);
    }
    Ok(LineType {
        action,
        recreate,
        boot,
        noerror,
        force,
        base64_decode,
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
            parse_cleanup_age, parse_duration, parse_duration_part, parse_line, CleanupParseError,
            FieldParseError, FileSpan, ParseError, MICROSECOND, SECOND, WEEK,
        },
    };

    #[test]
    fn test_duration_part() {
        assert_eq!(parse_duration_part(&mut b"1s".as_slice()), Ok(SECOND));
        assert_eq!(parse_duration_part(&mut "1µs".as_bytes()), Ok(MICROSECOND));
        assert_eq!(
            parse_duration_part(&mut "123456789".as_bytes()),
            Ok(SECOND * 123_456_789)
        );
    }

    #[test]
    fn test_duration() {
        assert_eq!(parse_duration(b"1s1m"), Ok(SECOND * 61));
        assert_eq!(
            parse_duration("6days23hr59m59sec999ms999µs1000ns".as_bytes()),
            Ok(WEEK)
        );
        assert_eq!(
            parse_cleanup_age(b"1s1m"),
            Ok(CleanupAge {
                age: SECOND * 61,
                ..CleanupAge::EMPTY
            })
        );
        assert_eq!(
            parse_cleanup_age("6days23hr59m59sec999ms999µs1000ns".as_bytes()),
            Ok(CleanupAge {
                age: WEEK,
                ..CleanupAge::EMPTY
            })
        );
    }

    #[test]
    fn test_line() {
        let dummy_file = Path::new("");
        assert_eq!(
            parse_line(FileSpan::from_slice(b"L+ /run/gdm/.config/pulse/default.pa - - - - /nix/store/whibfps24g91fx9i63m2wdyl87dfadnn-default.pa", dummy_file)),
            Ok(Line {
                line_type: Spanned::new(LineType { action: LineAction::CreateSymlink, recreate: true, boot: false, noerror: false, force: false, base64_decode: false }, dummy_file, 0..2 ),
                path: Spanned::new(PathBuf::from_str("/run/gdm/.config/pulse/default.pa").unwrap(), dummy_file, 3..36),
                mode: Spanned::new(None, dummy_file, 37..38),
                owner: Spanned::new(None, dummy_file, 39..40),
                group: Spanned::new(None, dummy_file, 41..42),
                age: Spanned::new(CleanupAge::EMPTY, dummy_file, 43..44),
                argument: Spanned::new(Some(OsString::from("/nix/store/whibfps24g91fx9i63m2wdyl87dfadnn-default.pa")), dummy_file, 45..99)
            })
        );
    }

    #[test]
    fn test_empty_line() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"", Path::new(""))),
            Err(FieldParseError::EndOfLine.into())
        )
    }

    #[test]
    fn test_illegal_parse_type() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"B", Path::new(""))),
            Err(ParseError::IllegalParseType(b'B'))
        )
    }
    #[test]
    fn test_tab() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\t", Path::new(""))),
            Err(ParseError::LeadingWhitespace)
        )
    }
    #[test]
    fn test_junk_after_quotes() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\"\"A", Path::new(""))),
            Err(FieldParseError::JunkAfterQuotes.into())
        )
    }
    #[test]
    fn test_empty_parse_type() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\"\"", Path::new(""))),
            Err(ParseError::EmptyParseType)
        )
    }
    #[test]
    fn test_trailing_backslash() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\\", Path::new(""))),
            Err(FieldParseError::TrailingBackslash.into())
        )
    }
    #[test]
    fn test_unrecognized_escape() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\\z", Path::new(""))),
            Err(FieldParseError::UnrecognizedEscape(b'z').into())
        )
    }
    #[test]
    fn test_invalid_type_combination() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z+", Path::new(""))),
            Err(ParseError::InvalidTypeCombination(b'Z', b'+'))
        )
    }
    #[test]
    fn test_invalid_type_modifier() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z\0", Path::new(""))),
            Err(ParseError::InvalidTypeModifier(b'\0'))
        )
    }
    #[test]
    fn test_invalid_mode_string() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"z z -x", Path::new(""))),
            Err(ParseError::InvalidMode)
        )
    }
    #[test]
    fn test_duplicate_type_modifier() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"A!!", Path::new(""))),
            Err(ParseError::DuplicateTypeModifier(b'!'))
        )
    }
    #[test]
    fn test_unsupported_octal_null() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\\0", Path::new(""))),
            Err(FieldParseError::UnsupportedOctalEscape.into())
        )
    }
    #[test]
    fn test_invalid_cleanup_age() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z - - - - f", Path::new(""))),
            Err(ParseError::InvalidCleanupAge(
                CleanupParseError::InvalidDurationInt(u64::from_str("").unwrap_err())
            ))
        )
    }
    #[test]
    fn test_unfinished_hex_escape() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\\x", Path::new(""))),
            Err(FieldParseError::UnfinishedHexEscape.into())
        )
    }
    #[test]
    fn test_invalid_username() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z - - \\xFF", Path::new(""))),
            Err(ParseError::InvalidUsername)
        )
    }
    #[test]
    fn test_invalid_hex_escape() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"\\xgg", Path::new(""))),
            Err(FieldParseError::InvalidHexEscape.into())
        )
    }
    #[test]
    fn test_null_in_path() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"z /\\x00", Path::new(""))),
            Err(ParseError::NullInPath)
        )
    }
    #[test]
    fn test_invalid_cleanup_specifier() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z / -	- - \0:", Path::new(""))),
            Err(ParseError::InvalidCleanupAge(
                CleanupParseError::InvalidCleanupSpecifier(b'\0')
            ))
        )
    }
    #[test]
    fn test_duplicate_cleanup_specifier() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z / -	- - AA:", Path::new(""))),
            Err(ParseError::InvalidCleanupAge(
                CleanupParseError::DuplicateCleanupSpecifier(b'A')
            ))
        )
    }
    #[test]
    fn test_malformed_cleanup() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z / -	- - AA::", Path::new(""))),
            Err(ParseError::InvalidCleanupAge(CleanupParseError::Malformed(
                b"AA::".as_slice().into()
            )))
        )
    }
    #[test]
    fn test_overflowed_cleanup_duration() {
        assert_eq!(
            parse_line(FileSpan::from_slice(
                b"Z	/ - - - 1s9999999999999month",
                Path::new("")
            )),
            Err(ParseError::InvalidCleanupAge(
                CleanupParseError::OverflowedDuration(b"9999999999999month".as_slice().into())
            ))
        )
    }
    #[test]
    fn test_empty_cleanup_specifiers() {
        assert_eq!(
            parse_line(FileSpan::from_slice(b"Z	/ - - - :1s", Path::new(""))),
            Err(ParseError::InvalidCleanupAge(
                CleanupParseError::EmptyCleanupSpecifierList
            ))
        )
    }
}
