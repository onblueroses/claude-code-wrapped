mod generator;
mod prototype;
mod sampler;

use generator::{
    append_incremental_tail, byte_identity, generate, relative_source_files, CorpusClass,
};
use sampler::MeasureOptions;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    if let Err(error) = run() {
        eprintln!("phase5-bench: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().ok_or_else(usage)?;
    match command.as_str() {
        "generate" => {
            let mut class = None;
            let mut output = None;
            let mut target_bytes = None;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--class" => {
                        class = Some(CorpusClass::parse(
                            &arguments
                                .next()
                                .ok_or_else(|| "--class requires a value".to_string())?,
                        )?);
                    }
                    "--output" => {
                        output = Some(PathBuf::from(
                            arguments
                                .next()
                                .ok_or_else(|| "--output requires a path".to_string())?,
                        ));
                    }
                    "--target-bytes" => {
                        let value = arguments
                            .next()
                            .ok_or_else(|| "--target-bytes requires a value".to_string())?;
                        target_bytes = Some(
                            value
                                .parse::<u64>()
                                .map_err(|_| format!("invalid target byte count `{value}`"))?,
                        );
                    }
                    _ => return Err(format!("unknown generate argument `{argument}`")),
                }
            }
            let class = class.ok_or_else(|| "generate requires --class".to_string())?;
            let output = output.ok_or_else(|| "generate requires --output".to_string())?;
            let summary = generate(class, &output, target_bytes)?;
            print!("{}", summary.manifest_json());
            Ok(())
        }
        "inventory" => {
            let root = one_path_argument(arguments, "--root")?;
            let files = relative_source_files(&root)?;
            for path in &files {
                println!("{}", path.display());
            }
            eprintln!("files={}", files.len());
            Ok(())
        }
        "compare" => {
            let arguments = arguments.collect::<Vec<_>>();
            let left = named_path(&arguments, "--left")?;
            let right = named_path(&arguments, "--right")?;
            if byte_identity(&left, &right)? {
                println!("byte_identity=PASS");
                Ok(())
            } else {
                Err("generated trees differ".to_string())
            }
        }
        "measure" => {
            let options = parse_measure_options(arguments)?;
            print!("{}", sampler::measure(options)?);
            Ok(())
        }
        "incremental-tail" => {
            let mut corpus = None;
            let mut output = None;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--corpus" => {
                        corpus = Some(PathBuf::from(next_value(&mut arguments, "--corpus")?));
                    }
                    "--output" => {
                        output = Some(PathBuf::from(next_value(&mut arguments, "--output")?));
                    }
                    _ => return Err(format!("unknown incremental-tail argument `{argument}`")),
                }
            }
            let corpus = corpus.ok_or_else(|| "incremental-tail requires --corpus".to_string())?;
            let output = output.ok_or_else(|| "incremental-tail requires --output".to_string())?;
            let summary = append_incremental_tail(&corpus, &output)?;
            print!("{summary}");
            Ok(())
        }
        "read-baseline" => {
            let mut corpus = None;
            let mut buffer_bytes = 1024 * 1024;
            let mut passes = 1;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--corpus" => {
                        corpus = Some(PathBuf::from(next_value(&mut arguments, "--corpus")?));
                    }
                    "--buffer-bytes" => {
                        buffer_bytes =
                            parse_usize(next_value(&mut arguments, "--buffer-bytes")?, "buffer")?;
                    }
                    "--passes" => {
                        passes = parse_usize(next_value(&mut arguments, "--passes")?, "passes")?;
                    }
                    _ => return Err(format!("unknown read-baseline argument `{argument}`")),
                }
            }
            let corpus = corpus.ok_or_else(|| "read-baseline requires --corpus".to_string())?;
            print!("{}", sampler::read_baseline(&corpus, buffer_bytes, passes)?);
            Ok(())
        }
        "memory-baseline" => {
            let mut workers = None;
            let mut bytes_per_worker = 64 * 1024 * 1024;
            let mut passes = 8;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--workers" => {
                        workers = Some(parse_usize(
                            next_value(&mut arguments, "--workers")?,
                            "worker count",
                        )?);
                    }
                    "--bytes-per-worker" => {
                        bytes_per_worker = parse_usize(
                            next_value(&mut arguments, "--bytes-per-worker")?,
                            "bytes per worker",
                        )?;
                    }
                    "--passes" => {
                        passes =
                            parse_usize(next_value(&mut arguments, "--passes")?, "memory passes")?;
                    }
                    _ => return Err(format!("unknown memory-baseline argument `{argument}`")),
                }
            }
            let workers =
                workers.ok_or_else(|| "memory-baseline requires --workers".to_string())?;
            print!(
                "{}",
                sampler::memory_baseline(workers, bytes_per_worker, passes)?
            );
            Ok(())
        }
        "sampler-overhead" => {
            let mut iterations = 1_000;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--iterations" => {
                        iterations = parse_usize(
                            next_value(&mut arguments, "--iterations")?,
                            "sampler iterations",
                        )?;
                    }
                    _ => return Err(format!("unknown sampler-overhead argument `{argument}`")),
                }
            }
            print!("{}", sampler::sampling_overhead(iterations)?);
            Ok(())
        }
        "sqlite-prototype" => {
            let mut mode = None;
            let mut binary = None;
            let mut corpus = None;
            let mut store = None;
            let mut scratch = None;
            let mut workers = None;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--mode" => {
                        mode = Some(match next_value(&mut arguments, "--mode")?.as_str() {
                            "first-import" => prototype::Mode::FirstImport,
                            "warm" => prototype::Mode::Warm,
                            value => return Err(format!("unknown prototype mode `{value}`")),
                        });
                    }
                    "--binary" => {
                        binary = Some(PathBuf::from(next_value(&mut arguments, "--binary")?));
                    }
                    "--corpus" => {
                        corpus = Some(PathBuf::from(next_value(&mut arguments, "--corpus")?));
                    }
                    "--store" => {
                        store = Some(PathBuf::from(next_value(&mut arguments, "--store")?));
                    }
                    "--scratch" => {
                        scratch = Some(PathBuf::from(next_value(&mut arguments, "--scratch")?));
                    }
                    "--workers" => {
                        workers = Some(parse_usize(
                            next_value(&mut arguments, "--workers")?,
                            "worker count",
                        )?);
                    }
                    _ => return Err(format!("unknown sqlite-prototype argument `{argument}`")),
                }
            }
            print!(
                "{}",
                prototype::run(prototype::Options {
                    mode: mode.ok_or_else(|| "sqlite-prototype requires --mode".to_string())?,
                    binary: binary
                        .ok_or_else(|| "sqlite-prototype requires --binary".to_string())?,
                    corpus: corpus
                        .ok_or_else(|| "sqlite-prototype requires --corpus".to_string())?,
                    store: store.ok_or_else(|| "sqlite-prototype requires --store".to_string())?,
                    scratch: scratch
                        .ok_or_else(|| "sqlite-prototype requires --scratch".to_string())?,
                    worker_count: workers
                        .ok_or_else(|| "sqlite-prototype requires --workers".to_string())?,
                })?
            );
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn parse_measure_options(
    mut arguments: impl Iterator<Item = String>,
) -> Result<MeasureOptions, String> {
    let mut binary = None;
    let mut corpus = None;
    let mut stderr = None;
    let mut scratch = None;
    let mut sample_millis = 10u64;
    let mut timeout_seconds = 300u64;
    let mut worker_count = None;
    let mut mode = None;
    let mut store = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--binary" => {
                binary = Some(PathBuf::from(next_value(&mut arguments, "--binary")?));
            }
            "--corpus" => {
                corpus = Some(PathBuf::from(next_value(&mut arguments, "--corpus")?));
            }
            "--stderr" => {
                stderr = Some(PathBuf::from(next_value(&mut arguments, "--stderr")?));
            }
            "--scratch" => {
                scratch = Some(PathBuf::from(next_value(&mut arguments, "--scratch")?));
            }
            "--sample-ms" => {
                sample_millis = parse_u64(
                    next_value(&mut arguments, "--sample-ms")?,
                    "sample interval",
                )?;
            }
            "--timeout-seconds" => {
                timeout_seconds =
                    parse_u64(next_value(&mut arguments, "--timeout-seconds")?, "timeout")?;
            }
            "--workers" => {
                worker_count = Some(parse_usize(
                    next_value(&mut arguments, "--workers")?,
                    "worker count",
                )?);
            }
            "--mode" => {
                mode = Some(next_value(&mut arguments, "--mode")?);
            }
            "--store" => {
                store = Some(PathBuf::from(next_value(&mut arguments, "--store")?));
            }
            _ => return Err(format!("unknown measure argument `{argument}`")),
        }
    }
    let store = match mode.as_deref() {
        Some("no-store") if store.is_none() => None,
        Some("store") => Some(store.ok_or_else(|| "store mode requires --store".to_string())?),
        Some("no-store") => return Err("no-store mode does not accept --store".to_string()),
        Some(value) => return Err(format!("unknown measure mode `{value}`")),
        None => return Err("measure requires --mode no-store|store".to_string()),
    };
    Ok(MeasureOptions {
        binary: binary.ok_or_else(|| "measure requires --binary".to_string())?,
        corpus: corpus.ok_or_else(|| "measure requires --corpus".to_string())?,
        stderr: stderr.ok_or_else(|| "measure requires --stderr".to_string())?,
        scratch: scratch.ok_or_else(|| "measure requires --scratch".to_string())?,
        sample_interval: Duration::from_millis(sample_millis),
        timeout: Duration::from_secs(timeout_seconds),
        worker_count,
        store,
    })
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_u64(value: String, name: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid {name} `{value}`"))
}

fn parse_usize(value: String, name: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid {name} `{value}`"))
}

fn one_path_argument(
    mut arguments: impl Iterator<Item = String>,
    name: &str,
) -> Result<PathBuf, String> {
    if arguments.next().as_deref() != Some(name) {
        return Err(format!("expected {name} PATH"));
    }
    let path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} requires a path"))?;
    if let Some(extra) = arguments.next() {
        return Err(format!("unexpected argument `{extra}`"));
    }
    Ok(path)
}

fn named_path(arguments: &[String], name: &str) -> Result<PathBuf, String> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or_else(|| format!("compare requires {name} PATH"))
}

fn usage() -> String {
    concat!(
        "usage: phase5-bench generate --class CLASS --output PATH [--target-bytes BYTES]\n",
        "       phase5-bench inventory --root PATH\n",
        "       phase5-bench compare --left PATH --right PATH\n",
        "       phase5-bench incremental-tail --corpus PATH --output PATH\n",
        "       phase5-bench sampler-overhead [--iterations N]\n",
        "       phase5-bench measure --binary PATH --corpus PATH --stderr PATH ",
        "--scratch PATH --mode no-store|store [--store PATH] ",
        "[--sample-ms N] [--timeout-seconds N] [--workers N]\n",
        "       phase5-bench read-baseline --corpus PATH [--buffer-bytes N] [--passes N]\n",
        "       phase5-bench memory-baseline --workers N ",
        "[--bytes-per-worker N] [--passes N]\n",
        "       phase5-bench sqlite-prototype --mode first-import|warm ",
        "--binary PATH --corpus PATH --store PATH --scratch PATH --workers N"
    )
    .to_string()
}
