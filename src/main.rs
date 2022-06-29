extern crate clap;

use std::str::FromStr;

use anyhow::{Error, Result};
use clap::{App, Arg, ArgMatches};
use ::http::header::{HeaderMap, HeaderName, HeaderValue};
use regex::Regex;
use tokio::time::Duration;
use std::io::prelude::*;

mod bench;
mod http;
mod results;
mod runtime;
mod utils;

use crate::http::BenchType;

/// Matches a string like '12d 24h 5m 45s' to a regex capture.
static DURATION_MATCH: &str =
    "(?P<days>[0-9]+)d|(?P<hours>[0-9]+)h|(?P<minutes>[0-9]+)m|(?P<seconds>[0-9]+)s";

/// ReWrk
///
/// Captures CLI arguments and build benchmarking settings and runtime to
/// suite the arguments and options.
fn main() {
    let args = parse_args();

    let threads: usize = match args.value_of("threads").unwrap_or("1").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "invalid parameter for 'threads' given, input type must be a integer."
            );
            return;
        },
    };

    let conns: usize = match args.value_of("connections").unwrap_or("1").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("invalid parameter for 'connections' given, input type must be a integer.");
            return;
        },
    };

    let host: &str = match args.value_of("host") {
        Some(v) => v,
        None => {
            eprintln!("missing 'host' parameter.");
            return;
        },
    };

    let headers: HeaderMap = match args.values_of("header") {
        Some(v) => {
            v.filter_map(|entry| {
                let parts = entry.splitn(2, ":").collect::<Vec<&str>>();
                if parts.len() == 2 { 
                    let k = HeaderName::from_str(&parts[0]);
                    let v = HeaderValue::from_str(&parts[1]);
                    if let (Ok(k),Ok(v)) = (k,v) {
                        Some((k,v))
                    } else { None }
                } else { None }
            }).fold(HeaderMap::new(), |mut acc, (k,v)| {
                acc.insert(k,v);
                acc
            })
        }
        None => HeaderMap::new()
    };

    let post: String = match args.value_of("post") {
        Some(v) => {
            let file = std::fs::File::open(v);
            let mut contents = String::new();
            if let Ok(mut file) = file {
                let _ = file.read_to_string(&mut contents);
            }
            contents
        },
        _ => String::new()
    };

    let http2: bool = args.is_present("http2");
    let json: bool = args.is_present("json");

    let bench_type = if http2 {
        BenchType::HTTP2
    } else {
        BenchType::HTTP1
    };

    let duration: &str = args.value_of("duration").unwrap_or("1s");
    let duration = match parse_duration(duration) {
        Ok(dur) => dur,
        Err(e) => {
            eprintln!("failed to parse duration parameter: {}", e);
            return;
        },
    };

    let pct: bool = args.is_present("pct");

    let rounds: usize = args
        .value_of("rounds")
        .unwrap_or("1")
        .parse::<usize>()
        .unwrap_or(1);

    let settings = bench::BenchmarkSettings {
        threads,
        connections: conns,
        host: host.to_string(),
        headers,
        post: post.to_owned(),
        bench_type,
        duration,
        display_percentile: pct,
        display_json: json,
        rounds,
    };

    bench::start_benchmark(settings);
}

/// Parses a duration string from the CLI to a Duration.
/// '11d 3h 32m 4s' -> Duration
///
/// If no matches are found for the string or a invalid match
/// is captured a error message returned and displayed.
fn parse_duration(duration: &str) -> Result<Duration> {
    let mut dur = Duration::default();

    let re = Regex::new(DURATION_MATCH).unwrap();
    for cap in re.captures_iter(duration) {
        let add_to = if let Some(days) = cap.name("days") {
            let days = days.as_str().parse::<u64>()?;

            let seconds = days * 24 * 60 * 60;
            Duration::from_secs(seconds)
        } else if let Some(hours) = cap.name("hours") {
            let hours = hours.as_str().parse::<u64>()?;

            let seconds = hours * 60 * 60;
            Duration::from_secs(seconds)
        } else if let Some(minutes) = cap.name("minutes") {
            let minutes = minutes.as_str().parse::<u64>()?;

            let seconds = minutes * 60;
            Duration::from_secs(seconds)
        } else if let Some(seconds) = cap.name("seconds") {
            let seconds = seconds.as_str().parse::<u64>()?;

            Duration::from_secs(seconds)
        } else {
            return Err(Error::msg(format!("invalid match: {:?}", cap)));
        };

        dur += add_to
    }

    if dur.as_secs() == 0 {
        return Err(Error::msg(format!(
            "failed to extract any valid duration from {}",
            duration
        )));
    }

    Ok(dur)
}

/// Contains Clap's app setup.
fn parse_args() -> ArgMatches<'static> {
    App::new("ReWrk")
        .version("0.3.3")
        .author("Harrison Burt <hburt2003@gmail.com>")
        .about("Benchmark HTTP/1 and HTTP/2 frameworks without pipelining bias.")
        .arg(
            Arg::with_name("threads")
                .short("t")
                .long("threads")
                .help("Set the amount of threads to use e.g. '-t 12'")
                .takes_value(true)
                .default_value("1"),
        )
        .arg(
            Arg::with_name("connections")
                .short("c")
                .long("connections")
                .help("Set the amount of concurrent e.g. '-c 512'")
                .takes_value(true)
                .default_value("1"),
        )
        .arg(
            Arg::with_name("host")
                .short("h")
                .long("host")
                .help("Set the host to bench e.g. '-h http://127.0.0.1:5050'")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("header")
                .short("H")
                .long("header")
                .help("Add HTTP header e.g. 'Content-Type: application/json'")
                .takes_value(true)
                .multiple(true)
                .required(false)
                .min_values(0),
        )
        .arg(
            Arg::with_name("post")
                .long("post")
                .help("Set the POST data using a file e.g. '--post ./my-data'")
                .takes_value(true)
                .required(false)
        )
        .arg(
            Arg::with_name("http2")
                .long("http2")
                .help("Set the client to use http2 only. (default is http/1) e.g. '--http2'")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("duration")
                .short("d")
                .long("duration")
                .help("Set the duration of the benchmark.")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("pct")
                .long("pct")
                .help("Displays the percentile table after benchmarking.")
                .takes_value(false)
                .required(false),
        )
        .arg(
            Arg::with_name("json")
                .long("json")
                .help("Displays the results in a json format")
                .takes_value(false)
                .required(false),
        )
        .arg(
            Arg::with_name("rounds")
                .long("rounds")
                .help("Repeats the benchmarks n amount of times")
                .takes_value(true)
                .required(false),
        )
        //.arg(
        //    Arg::with_name("random")
        //        .long("rand")
        //        .help(
        //            "Sets the benchmark type to random mode, \
        //             clients will randomly connect and re-connect.\n\
        //             NOTE: This will cause the HTTP2 flag to be ignored."
        //        )
        //        .takes_value(false)
        //        .required(false)
        //)
        .get_matches()
}
