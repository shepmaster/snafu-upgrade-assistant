use argh::FromArgs;
use itertools::Itertools;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
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
    const ERROR_CODES: &'static [&'static str] = &["E0422", "E0423", "E0425", "E0432"];

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

    /// what context selector suffix to use. Defaults to "Context"
    #[argh(option, default = "Self::DEFAULT_SUFFIX.to_string()")]
    suffix: String,

    /// what directory to make changes in. Defaults to the current
    /// directory
    #[argh(option, default = "env::current_dir().unwrap()")]
    directory: PathBuf,

    /// how many iterations to perform before giving up
    #[argh(option, default = "Self::DEFAULT_MAXIMUM_ITERATIONS")]
    max_iterations: usize,
}

impl Opts {
    const DEFAULT_SUFFIX: &'static str = "Context";
    const DEFAULT_MAXIMUM_ITERATIONS: usize = 5;
}

fn main() -> Result<()> {
    let opts: Opts = argh::from_env();

    if opts.version {
        println!(env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut depth = 0;
    let mut last_fix = apply_once(&opts)?;

    if opts.dry_run {
        return Ok(());
    }

    loop {
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
    let output = Command::new("cargo")
        .arg("build")
        .args(&["--message-format", "json"])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;

    // dbg!(&stdout);

    let lines: Vec<Line> = stdout
        .lines()
        .map(|l| serde_json::from_str(l))
        .collect::<Result<_, _>>()?;

    // dbg!(&lines);

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

    // dbg!(&relevant_spans);

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

    // dbg!(&file_mapping);

    for (filename, spans) in &mut file_mapping {
        let filename = Path::new(filename);
        if !filename.starts_with(&opts.directory) {
            return Err(format!(
                "Attempted to update file outside of safe directory. {} is not within {}",
                filename.display(),
                opts.directory.display(),
            )
            .into());
        }

        // dbg!(filename);

        let content = fs::read_to_string(filename)?;
        let mut content: &str = &content;

        let mut pieces = Vec::new();

        for (_start, end) in spans.iter().copied().rev() {
            let (head, tail) = content.split_at(end);

            // dbg!(&head[head.len() - 10..]);
            // dbg!(&tail[..10]);

            // Assume we've already applied the suffix and avoid adding it again
            if head.ends_with(&opts.suffix) {
                continue;
            }

            let head = head.strip_suffix("Error").unwrap_or(head);

            pieces.push(tail);
            content = head;
        }
        pieces.push(content);

        // dbg!(spans.len(), pieces.len());

        let modified_content: String =
            Itertools::intersperse(pieces.iter().copied().rev(), &opts.suffix).collect();
        // dbg!(&modified_content);

        if opts.dry_run {
            eprintln!("Would write modified content to '{}'", filename.display());
        } else {
            fs::write(filename, modified_content)?;
        }
    }

    Ok(file_mapping)
}
