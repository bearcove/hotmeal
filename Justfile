list:
    just --list

test:
    cargo nextest run --no-fail-fast
    cd hotmeal-wasm && pnpm test
