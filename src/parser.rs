use std::ffi::OsString;
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};

use nom::branch::alt;
use nom::bytes::complete::{take_till, take_till1};
use nom::character::complete::{anychar, char};
use nom::combinator::{all_consuming, fail};
use nom::sequence::tuple;
use nom::{
    bytes::complete::{tag, take_while},
    character::{
        complete::{digit1, one_of},
        is_alphabetic,
    },
    combinator::{map_opt, map_res, opt},
    multi::{fold_many1, many1},
    sequence::pair,
    IResult,
};
use nom::{AsChar, FindToken, InputTakeAtPosition};
use phf::phf_map;

use crate::config_file::{CleanupAge, FileOwner, Line, LineAction, LineType};

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
    let (input, count) = map_res(map_res(digit1, std::str::from_utf8), u64::from_str)(input)?;
    let (input, unit) = map_opt(
        take_while(|chr| is_alphabetic(chr) || chr >= 0x80),
        |name| DURATION_KEYWORDS.get(name),
    )(input)?;

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
    fold_many1(
        parse_duration_part,
        Duration::default,
        Duration::saturating_add,
    )(input)
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

fn parse_cleanup_age(input: &[u8]) -> IResult<&[u8], CleanupAge> {
    let (input, maybe_age_by) = opt(pair(parse_cleanup_age_by, tag(b":")))(input)?;
    let (input, duration) = parse_duration(input)?;

    let mut cleanup_age = match maybe_age_by {
        Some(result) => result.0,
        None => CleanupAge::EMPTY,
    };

    cleanup_age.age = duration;

    Ok((input, cleanup_age))
}

fn line_space(input: &[u8]) -> IResult<&[u8], &[u8]> {
    input.split_at_position1_complete(
        |item| {
            let c = item.as_char();
            !matches!(c, ' ' | '\t')
        },
        nom::error::ErrorKind::Alpha,
    )
}

fn parse_line(input: &[u8]) -> IResult<&[u8], Line> {
    let (input, (line_type, _, path, _, mode, _, owner, _, group, _, age, _, argument)) =
        tuple((
            parse_type,
            line_space,
            parse_path,
            line_space,
            parse_mode,
            line_space,
            parse_owner,
            line_space,
            parse_group,
            line_space,
            parse_cleanup_age,
            line_space,
            parse_argument,
        ))(input)?;

    Ok((
        input,
        Line {
            line_type,
            path,
            mode,
            owner,
            group,
            age,
            argument,
        },
    ))
}

fn parse_mode(input: &[u8]) -> IResult<&[u8], Option<u32>> {
    todo!()
}
fn parse_argument(input: &[u8]) -> IResult<&[u8], Option<OsString>> {
    todo!()
}
fn parse_owner(input: &[u8]) -> IResult<&[u8], Option<FileOwner>> {
    todo!()
}
fn parse_group(input: &[u8]) -> IResult<&[u8], Option<FileOwner>> {
    todo!()
}

fn parse_path(input: &[u8]) -> IResult<&[u8], PathBuf> {
    todo!()
}

fn quoted(input: &[u8]) -> IResult<&[u8], &[u8]> {
    let (input, quote) = one_of(b"'\"".as_slice())(input)?;
    let mut input = input;
    let mut index = 0;
    loop {
        match char::from(input[index]) {
            c @ ('\'' | '"') if c == quote => break,
            '\\' => index += 2,
            '\n' => {
                fail::<_, (), _>(input)?;
            }
            _ => index += 1,
        }
    }
    let (input, string) = input.split_at(index);
    let (input, _) = char(quote)(input)?;
    Ok((input, string))
}

fn unescape(input: &[u8]) -> IResult<&[u8], Box<[u8]>> {
    let (input, string) = alt((quoted, take_till1(|c| matches!(c, b' ' | b'\t'))))(input)?;
    todo!()
}

fn parse_type(input: &[u8]) -> IResult<&[u8], LineType> {
    let (input, unescaped) = unescape(input)?;
    let unescaped = &*unescaped;
    let (_, (char, modifiers)) = match all_consuming(tuple((anychar::<_, nom::error::Error<_>>, take_till(|c| char::from(c).is_whitespace()))))(unescaped) {
        Ok((char, modifiers)) => (char, modifiers),
        Err(_) => todo!(),
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
    Ok((
        input,
        LineType {
            action,
            recreate,
            boot,
            noerror,
            force,
        },
    ))
}

#[cfg(test)]
mod test {
    use crate::parser::{
        parse_duration, parse_duration_part, MICROSECOND, NANOSECOND, SECOND, WEEK,
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
}
