use std::str::FromStr;
use std::time::Duration;

use nom::{
    bytes::complete::tag,
    bytes::complete::take_while,
    character::{
        complete::{digit1, one_of},
        is_alphabetic,
    },
    combinator::{map_opt, map_res, opt},
    multi::{fold_many1, many1},
    sequence::pair,
    IResult,
};
use phf::phf_map;

use crate::config_file::CleanupAge;

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
