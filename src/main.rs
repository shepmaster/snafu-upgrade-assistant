use argh::FromArgs;
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
    fn is_context_selector_rename(&self) -> bool {
        let error_codes = &["E0412", "E0422", "E0423", "E0425", "E0432", "E0574"];

        error_codes.iter().any(|&c| c == self.code)
    }

    fn is_with_context_argument(&self) -> bool {
        let error_codes = &["E0593"];
        error_codes.iter().any(|&c| c == self.code)
    }

    fn categorize<T>(&self, v: T) -> Option<Category<T>> {
        if self.is_context_selector_rename() {
            Some(Category::ContextSelectorRename(v))
        } else if self.is_with_context_argument() {
            Some(Category::WithContextArgument(v))
        } else {
            None
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Category<T> {
    ContextSelectorRename(T),
    WithContextArgument(T),
}

impl<T> Category<T> {
    fn as_ref(&self) -> Category<&T> {
        use Category::*;

        match self {
            ContextSelectorRename(v) => ContextSelectorRename(v),
            WithContextArgument(v) => WithContextArgument(v),
        }
    }

    fn unify(self) -> T {
        use Category::*;

        match self {
            ContextSelectorRename(v) => v,
            WithContextArgument(v) => v,
        }
    }

    fn map<U>(self, f: impl FnOnce(T) -> U) -> Category<U> {
        use Category::*;

        match self {
            ContextSelectorRename(v) => ContextSelectorRename(f(v)),
            WithContextArgument(v) => WithContextArgument(f(v)),
        }
    }
}

impl<T> IntoIterator for Category<T>
where
    T: IntoIterator,
{
    type Item = Category<T::Item>;
    type IntoIter = std::iter::Map<T::IntoIter, fn(T::Item) -> Category<T::Item>>;

    fn into_iter(self) -> Self::IntoIter {
        use Category::*;

        match self {
            ContextSelectorRename(v) => v.into_iter().map(ContextSelectorRename),
            WithContextArgument(v) => v.into_iter().map(WithContextArgument),
        }
    }
}

impl<T> PartialOrd for Category<T>
where
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_ref().unify().partial_cmp(other.as_ref().unify())
    }
}

impl<T> Ord for Category<T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_ref().unify().cmp(other.as_ref().unify())
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

type FileMapping = BTreeMap<String, Vec<Category<(usize, usize)>>>;

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
        .flat_map(|m| m.code.as_ref().and_then(|c| c.categorize(m)))
        .flat_map(|c| c.map(|m| &m.spans))
        .filter(|c| match c {
            Category::ContextSelectorRename(v) => v.is_primary,
            // The secondary error message points to the closure argument
            Category::WithContextArgument(v) => !v.is_primary,
        })
        .collect();

    if opts.verbose {
        dbg!(&relevant_spans);
    }

    let mut file_mapping: FileMapping = BTreeMap::new();
    for &span in &relevant_spans {
        let file_name = &span.as_ref().unify().file_name;
        let range = span.as_ref().map(|s| (s.byte_start, s.byte_end));
        file_mapping
            .entry(file_name.to_owned())
            .or_default()
            .push(range);
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

        let mut pieces: Vec<&str> = Vec::new();

        for cat in spans.iter().copied().rev() {
            match cat {
                Category::ContextSelectorRename((_start, end)) => {
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
                    pieces.push(&opts.suffix);
                    content = head;
                }
                Category::WithContextArgument((_start, end)) => {
                    // The range points to the `||` in the closure
                    // arguments, so we offset by one to get into the
                    // middle
                    let (head, tail) = content.split_at(end - 1);

                    if opts.verbose {
                        dbg!(&head[head.len() - 10..]);
                        dbg!(&tail[..10]);
                    }

                    pieces.push(tail);
                    pieces.push("_");
                    content = head;
                }
            }
        }
        pieces.push(content);

        if opts.verbose {
            dbg!(spans.len(), pieces.len());
        }

        let modified_content: String = pieces.iter().copied().rev().collect();

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
