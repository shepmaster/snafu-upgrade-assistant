#!/usr/bin/env bash

set -eu

function run_one_in_place() (
    trap 'git checkout -- . ../Cargo.lock' EXIT

    cargo run --quiet --manifest-path=../../Cargo.toml
    sed -e 's/0\.6/0.7.0-beta.2/' Cargo.toml > Cargo.post.toml

    mv Cargo.post.toml Cargo.toml
    cargo update --quiet
    cargo run --quiet --manifest-path=../../Cargo.toml

    # Successful build?
    cargo build --quiet

    # Looks like we want?
    diff -q src/main.rs src/main.rs.expected
)

function run_one() (
    trap 'popd' EXIT

    pushd "${1}"
    run_one_in_place
)

function run_all() (
    for d in test-*; do
        run_one "${d}"
    done
)

run_all
