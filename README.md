# snafu-upgrade-assistant

Upgrades usages of [SNAFU][] 0.6 to 0.7.

[SNAFU]: https://crates.io/crates/snafu

## TL;DR

1. Install the assistant

    ```
    cargo install snafu-upgrade-assistant
    ```

1. Run the assistant inside of your Cargo project

    ```
    snafu-upgrade-assistant
    ```

    This should compile successfully and make no changes to your files.

1. Update SNAFU from 0.6 to 0.7 in your Cargo.toml

1. Run the assistant again

1. Commit changes and run tests

## What's going on?

In SNAFU 0.7, generated *context selectors* now have the `Snafu`
suffix to help de-mystify the generated code. This tool builds your
code, looks at the compiler error messages, and applies automated
transformations to try to get it building again.

## What options exist?

Run the assistant with `--help` for the complete list of options. Some
commonly used ones are:

- `--dry-run`. When set, the assistant will do one iteration of fixes
  and print out what files would be modified.

- `--extra-check-arg`. When provided, the assistant will use these
  extra arguments to `cargo check`. Can be used more than once. Useful
  for passing feature flags (`--extra-check-arg --feature=cool-thing`)
  or workspace related configuration (`--extra-check-arg --all`).

## Is this safe?

The assistant is designed to only change files inside of the current
working directory that the Rust compiler reports errors have occurred
in. That said, you should always start work with a clean version
control state, and it doesn't hurt to have a backup of your directory.
