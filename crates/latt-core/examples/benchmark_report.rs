//! Normalize pinned Criterion output into Lattice's versioned benchmark record.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use serde_json::{Value, json};

#[path = "../benches/operations/mod.rs"]
mod operations;
use operations::{OPERATIONS, Operation, ScaleSet, ThroughputBasis};

const SCHEMA_VERSION: &str = "2.2";

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    None,
    Saved,
    Compared,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Saved => "saved",
            Self::Compared => "compared",
        }
    }
}

#[derive(Parser)]
#[command(about = "Normalize pinned Criterion output into Lattice's versioned benchmark record")]
struct Args {
    criterion_dir: PathBuf,
    fixture_manifest: PathBuf,
    output: PathBuf,
    /// How the benchmark run handled its baseline.
    #[arg(long, value_enum, default_value_t = Mode::None)]
    mode: Mode,
    /// The baseline name the run saved, compared against, or asked for and did not find.
    #[arg(long)]
    baseline: Option<String>,
}

fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!("{program} returned {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|error| format!("{program} output was not UTF-8: {error}"))
}

/// Map every `full_id` Criterion wrote under a `new/` directory to that directory.
fn visit_runs(directory: &Path, runs: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?
    {
        let path = entry
            .map_err(|error| format!("could not read {} entry: {error}", directory.display()))?
            .path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == "new") {
            let benchmark_path = path.join("benchmark.json");
            let benchmark = read_json(&benchmark_path)?;
            let Some(id) = benchmark["full_id"].as_str() else {
                return Err(format!(
                    "{} has no string full_id",
                    benchmark_path.display()
                ));
            };
            if let Some(first) = runs.insert(id.to_owned(), path.clone()) {
                return Err(format!(
                    "{id} is recorded twice, under {} and {}",
                    first.display(),
                    path.display()
                ));
            }
        } else {
            visit_runs(&path, runs)?;
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn number(value: &Value, what: &str, path: &Path) -> Result<f64, String> {
    value
        .as_f64()
        .ok_or_else(|| format!("{} has no numeric {what}", path.display()))
}

fn interval(value: &Value, name: &str, unit: &str, path: &Path) -> Result<Value, String> {
    let estimate = value
        .get(name)
        .ok_or_else(|| format!("{} has no {name} estimate", path.display()))?;
    let confidence = &estimate["confidence_interval"];
    Ok(json!({
        format!("point_estimate_{unit}"): number(&estimate["point_estimate"], &format!("{name} point estimate"), path)?,
        format!("standard_error_{unit}"): number(&estimate["standard_error"], &format!("{name} standard error"), path)?,
        "confidence_interval": {
            "confidence_level": number(&confidence["confidence_level"], &format!("{name} confidence level"), path)?,
            format!("lower_bound_{unit}"): number(&confidence["lower_bound"], &format!("{name} lower bound"), path)?,
            format!("upper_bound_{unit}"): number(&confidence["upper_bound"], &format!("{name} upper bound"), path)?
        }
    }))
}

fn fences(run_dir: &Path) -> Result<Value, String> {
    let path = run_dir.join("tukey.json");
    let fences = read_json(&path)?;
    let values = fences
        .as_array()
        .filter(|values| values.len() == 4)
        .ok_or_else(|| format!("{} is not an array of four fences", path.display()))?;
    values
        .iter()
        .map(|value| number(value, "fence", &path))
        .collect::<Result<Vec<f64>, String>>()
        .map(Value::from)
}

/// Cross-check the throughput the harness recorded against the shared declaration
/// and the scale's node count.
fn throughput(run_dir: &Path, operation: &Operation, scale: &Scale) -> Result<Value, String> {
    let path = run_dir.join("benchmark.json");
    let recorded = &read_json(&path)?["throughput"];
    match operation.throughput {
        Some(ThroughputBasis::Nodes) => {
            let elements = recorded["Elements"].as_u64().ok_or_else(|| {
                format!(
                    "{} declares no element throughput but the operation list does",
                    path.display()
                )
            })?;
            if elements != scale.nodes {
                return Err(format!(
                    "{} records {elements} elements but scale {} has {} nodes",
                    path.display(),
                    scale.label,
                    scale.nodes
                ));
            }
            Ok(json!({ "basis": "nodes", "elements": elements }))
        }
        None if recorded.is_null() => Ok(Value::Null),
        None => Err(format!(
            "{} records a throughput the operation list does not declare",
            path.display()
        )),
    }
}

/// Read the comparison Criterion wrote, by mode: never for a plain run, when present
/// after a save, and always for a compared benchmark the named baseline covers.
fn change(run_dir: &Path, args: &Args) -> Result<Value, String> {
    let benchmark_dir = run_dir.parent().expect("run directory has a parent");
    let path = benchmark_dir.join("change").join("estimates.json");
    let covered = args
        .baseline
        .as_deref()
        .is_some_and(|name| benchmark_dir.join(name).join("estimates.json").is_file());
    match (args.mode, path.is_file()) {
        (Mode::None, _) | (Mode::Saved, false) => Ok(Value::Null),
        (Mode::Compared, false) if !covered => Ok(Value::Null),
        (Mode::Compared, false) => Err(format!(
            "{} is missing although the baseline covers this benchmark",
            path.display()
        )),
        (_, true) => {
            let estimates = read_json(&path)?;
            Ok(json!({
                "mean": interval(&estimates, "mean", "ratio", &path)?,
                "median": interval(&estimates, "median", "ratio", &path)?
            }))
        }
    }
}

fn measurement(
    run_dir: &Path,
    operation: &Operation,
    scale: &Scale,
    args: &Args,
) -> Result<Value, String> {
    let path = run_dir.join("estimates.json");
    let estimates = read_json(&path)?;
    Ok(json!({
        "mean": interval(&estimates, "mean", "nanoseconds", &path)?,
        "median": interval(&estimates, "median", "nanoseconds", &path)?,
        "std_dev": interval(&estimates, "std_dev", "nanoseconds", &path)?,
        "median_abs_dev": interval(&estimates, "median_abs_dev", "nanoseconds", &path)?,
        "tukey_fences_nanoseconds": fences(run_dir)?,
        "throughput": throughput(run_dir, operation, scale)?,
        "body": operation.body,
        "timing": {
            "setup": operation.timing.setup.as_str(),
            "input_drop": operation.timing.input_drop.as_str(),
            "output_drop": operation.timing.output_drop.as_str()
        },
        "change": change(run_dir, args)?
    }))
}

struct Scale {
    label: String,
    nodes: u64,
}

/// Read every declared scale, graph and adapter, refusing a label declared twice
/// across both lists or one without an integer node count.
fn scales(manifest: &Value) -> Result<Vec<(Scale, Option<String>)>, String> {
    let mut scales: Vec<(Scale, Option<String>)> = Vec::new();
    let adapter_scales = manifest["adapter_scales"]
        .as_array()
        .ok_or_else(|| "fixture manifest adapter_scales must be an array".to_owned())?;
    let graph_scales = manifest["scales"]
        .as_array()
        .ok_or_else(|| "fixture manifest scales must be an array".to_owned())?;
    for (scale, adapter_list) in graph_scales
        .iter()
        .map(|scale| (scale, false))
        .chain(adapter_scales.iter().map(|scale| (scale, true)))
    {
        let label = scale["label"]
            .as_str()
            .ok_or_else(|| "fixture scale label must be a string".to_owned())?;
        if scales.iter().any(|(seen, _)| seen.label == label) {
            return Err(format!("fixture scale label {label} is declared twice"));
        }
        let nodes = scale["nodes"]
            .as_u64()
            .ok_or_else(|| format!("fixture scale {label} has no integer node count"))?;
        let adapter = if adapter_list {
            Some(
                scale["adapter"]
                    .as_str()
                    .ok_or_else(|| format!("adapter scale {label} names no adapter"))?
                    .to_owned(),
            )
        } else {
            None
        };
        scales.push((
            Scale {
                label: label.to_owned(),
                nodes,
            },
            adapter,
        ));
    }
    Ok(scales)
}

/// The scales `operation` runs at: every graph scale, or the adapter scales naming
/// its adapter, of which there must be at least one.
fn scales_for<'s>(
    scales: &'s [(Scale, Option<String>)],
    operation: &Operation,
) -> Result<Vec<&'s Scale>, String> {
    let applies = scales
        .iter()
        .filter(|(_, adapter)| match operation.scales {
            ScaleSet::Graph => adapter.is_none(),
            ScaleSet::Adapter(name) => adapter.as_deref() == Some(name),
        })
        .map(|(scale, _)| scale)
        .collect::<Vec<_>>();
    if applies.is_empty() {
        return Err(format!(
            "fixture manifest declares no scale for {}/{}",
            operation.group, operation.function
        ));
    }
    Ok(applies)
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|rest| rest.split_once(':'))
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn runner_class() -> &'static str {
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        "github-actions"
    } else if std::env::var_os("GITLAB_CI").is_some() {
        "gitlab-ci"
    } else {
        "local"
    }
}

fn criterion_version() -> Result<String, String> {
    let metadata: Value = serde_json::from_str(&command_output(
        "cargo",
        &["metadata", "--format-version", "1", "--locked"],
    )?)
    .map_err(|error| format!("cargo metadata output was not JSON: {error}"))?;
    metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages.iter().find_map(|package| {
                (package["name"].as_str() == Some("criterion"))
                    .then(|| package["version"].as_str().map(str::to_owned))
                    .flatten()
            })
        })
        .ok_or_else(|| "cargo metadata did not contain criterion".to_owned())
}

fn normalize(args: &Args, manifest: Value) -> Result<Value, String> {
    let scales = scales(&manifest)?;
    let mut runs = BTreeMap::new();
    visit_runs(&args.criterion_dir, &mut runs)?;
    let mut measurements = BTreeMap::new();
    let mut missing = Vec::new();
    for operation in &OPERATIONS {
        for scale in scales_for(&scales, operation)? {
            let id = format!("{}/{}/{}", operation.group, operation.function, scale.label);
            // Removed as matched, so what remains ran but is declared nowhere.
            match runs.remove(&id) {
                Some(run_dir) => {
                    measurements.insert(id, measurement(&run_dir, operation, scale, args)?);
                }
                None => missing.push(id),
            }
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Criterion results are incomplete; missing benchmark IDs: {}",
            missing.join(", ")
        ));
    }
    if !runs.is_empty() {
        let undeclared: Vec<&str> = runs.keys().map(String::as_str).collect();
        return Err(format!(
            "Criterion results hold undeclared benchmark IDs: {}",
            undeclared.join(", ")
        ));
    }
    let generated_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?
        .as_secs();
    let status = command_output(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "generated_unix_seconds": generated_unix_seconds,
        "source": {
            "commit": command_output("git", &["rev-parse", "HEAD"])?,
            "working_tree_dirty": !status.is_empty()
        },
        "toolchain": {
            "rustc": command_output("rustc", &["--version", "--verbose"])?,
            "criterion": criterion_version()?
        },
        "environment": {
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "cpu": cpu_model(),
            "runner_class": runner_class()
        },
        "lattice_features": "default",
        "baseline": {
            "requested": args.baseline,
            "mode": args.mode.as_str()
        },
        "fixture": manifest,
        "measurements": measurements
    }))
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    if args.mode != Mode::None && args.baseline.is_none() {
        return Err(format!(
            "--mode {} needs --baseline <name>",
            args.mode.as_str()
        ));
    }
    let manifest = read_json(&args.fixture_manifest)?;
    let report = normalize(&args, manifest)?;
    let parent = args
        .output
        .parent()
        .ok_or_else(|| format!("{} has no parent", args.output.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let encoded = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not encode benchmark report: {error}"))?;
    fs::write(&args.output, format!("{encoded}\n"))
        .map_err(|error| format!("could not write {}: {error}", args.output.display()))?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}
