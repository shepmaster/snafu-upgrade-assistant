use argh::FromArgs;
use itertools::Itertools;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
    process::{self, Command},
};

type Error = Box<dyn std::error::Error>;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Deserialize)]
#[serde(tag = "reason")]
enum Line {
    #[serde(rename = "compiler-message")]
    CompilerMessage { message: Message },

    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Message {
    code: Option<Code>,
    spans: Vec<Span>,
}

#[derive(Debug, Deserialize)]
struct Code {
    code: String,
}

impl Code {
    const ERROR_CODES: &'static [&'static str] =
        &["E0412", "E0422", "E0423", "E0425", "E0432", "E0574"];

    fn is_relevant(&self) -> bool {
        Self::ERROR_CODES.iter().any(|&c| c == self.code)
    }
}

#[derive(Debug, Deserialize, PartialOrd, Ord, PartialEq, Eq)]
struct Span {
    byte_start: usize,
    byte_end: usize,
    file_name: String,
    is_primary: bool,
}

/// Helps upgrade SNAFU between semver-incompatible versions
#[derive(Debug, FromArgs)]
struct Opts {
    /// show version information
    #[argh(switch)]
    version: bool,

    /// do not write changes to disk
    #[argh(switch)]
    dry_run: bool,

    /// extra arguments to `cargo check`. The option may be used
    /// multiple times.
    #[argh(option)]
    extra_check_arg: Vec<String>,

    /// what context selector suffix to use. Defaults to "Snafu"
    #[argh(option, default = "Self::DEFAULT_SUFFIX.to_string()")]
    suffix: String,

    /// what directory to make changes in. Defaults to the workspace
    /// root
    #[argh(option, default = "workspace_root().unwrap()")]
    directory: PathBuf,

    /// how many iterations to perform before giving up
    #[argh(option, default = "Self::DEFAULT_MAXIMUM_ITERATIONS")]
    max_iterations: usize,

    /// show detailed information
    #[argh(switch)]
    verbose: bool,
}

impl Opts {
    const DEFAULT_SUFFIX: &'static str = "Snafu";
    const DEFAULT_MAXIMUM_ITERATIONS: usize = 5;
}

fn main() -> Result<()> {
    let opts: Opts = argh::from_env();

    if opts.version {
        println!(env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    eprintln!("Performing initial check build; this may take a while");

    let mut depth = 0;
    let mut last_fix = apply_once(&opts)?;

    if opts.dry_run {
        return Ok(());
    }

    loop {
        if opts.verbose {
            dbg!(depth);
        }

        if last_fix.is_empty() {
            break;
        }

        if depth > opts.max_iterations {
            eprintln!(
                "Could not converge on a resolution in {} attempts",
                opts.max_iterations
            );
            process::exit(1);
        }

        eprintln!("Performing follow-up check build");
        let current_fix = apply_once(&opts)?;

        if last_fix == current_fix {
            eprintln!("Did not make progress on a resolution");
            process::exit(1);
        }

        last_fix = current_fix;
        depth += 1;
    }

    Ok(())
}

type FileMapping = BTreeMap<String, Vec<(usize, usize)>>;

fn apply_once(opts: &Opts) -> Result<FileMapping> {
    let mut build_command = Command::new("cargo");
    build_command.arg("check");
    for arg in &opts.extra_check_arg {
        build_command.arg(arg);
    }
    build_command.args(&["--message-format", "json"]);

    if opts.verbose {
        dbg!(&build_command);
    }

    let output = build_command.output()?;
    let stdout = String::from_utf8(output.stdout)?;

    if opts.verbose {
        dbg!(&stdout);
    }

    let lines: Vec<Line> = stdout
        .lines()
        .map(|l| serde_json::from_str(l))
        .collect::<Result<_, _>>()?;

    if opts.verbose {
        dbg!(&lines);
    }

    let relevant_spans: BTreeSet<_> = lines
        .iter()
        .flat_map(|l| match l {
            Line::CompilerMessage { message } => Some(message),
            Line::Other => None,
        })
        .filter(|m| m.code.as_ref().map_or(false, Code::is_relevant))
        .flat_map(|m| &m.spans)
        .filter(|s| s.is_primary)
        .collect();

    if opts.verbose {
        dbg!(&relevant_spans);
    }

    let mut file_mapping: FileMapping = BTreeMap::new();
    for &span in &relevant_spans {
        let Span {
            byte_end,
            byte_start,
            ref file_name,
            is_primary: _,
        } = *span;
        file_mapping
            .entry(file_name.to_owned())
            .or_default()
            .push((byte_start, byte_end));
    }

    if opts.verbose {
        dbg!(&file_mapping);
    }

    let workspace_root = workspace_root()?;

    for (filename, spans) in &mut file_mapping {
        let filename = workspace_root.join(filename);
        if !filename.starts_with(&opts.directory) {
            return Err(format!(
                "Attempted to update file outside of safe directory. {} is not within {}",
                filename.display(),
                opts.directory.display(),
            )
            .into());
        }

        if opts.verbose {
            dbg!(&filename);
        }

        let content = fs::read_to_string(&filename)?;
        let mut content: &str = &content;

        let mut pieces = Vec::new();

        for (_start, end) in spans.iter().copied().rev() {
            let (head, tail) = content.split_at(end);

            if opts.verbose {
                dbg!(&head[head.len() - 10..]);
                dbg!(&tail[..10]);
            }

            // Assume we've already applied the suffix and avoid adding it again
            if head.ends_with(&opts.suffix) {
                continue;
            }

            let head = head.strip_suffix("Error").unwrap_or(head);
            let head = head.strip_suffix("Context").unwrap_or(head);

            pieces.push(tail);
            content = head;
        }
        pieces.push(content);

        if opts.verbose {
            dbg!(spans.len(), pieces.len());
        }

        let modified_content: String =
            Itertools::intersperse(pieces.iter().copied().rev(), &opts.suffix).collect();

        if opts.verbose {
            dbg!(&modified_content);
        }

        if opts.dry_run {
            eprintln!("Would write modified content to '{}'", filename.display());
        } else {
            fs::write(&filename, modified_content)?;
        }
    }

    Ok(file_mapping)
}

fn workspace_root() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(&["metadata", "--format-version", "1"])
        .output()?;
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;

    Ok(metadata.workspace_root.into())
}

#[derive(Debug, Deserialize)]
struct Metadata {
    workspace_root: String,
}
