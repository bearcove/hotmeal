# Fuzzing targets for hotmeal

# Run the diff/apply roundtrip fuzzer with HTML dictionary
fuzz-browser *ARGS:
    just fuzz-prep
    cd hotmeal/fuzz && cargo fuzz run browser -- -dict=html.dict {{ ARGS }}

# Run the diff/apply roundtrip fuzzer with HTML dictionary
fuzz-mutate *ARGS:
    just fuzz-prep
    cd hotmeal/fuzz && cargo fuzz run mutate -- -dict=html.dict {{ ARGS }}

# Minimize a crash artifact
tmin-mutate ARTIFACT:
    just fuzz-prep
    cd hotmeal/fuzz && cargo fuzz tmin mutate {{ ARTIFACT }}

# Run with coverage report
fuzz-mutate-cov *ARGS:
    just fuzz-prep
    cd hotmeal/fuzz && cargo fuzz coverage mutate -- -dict=html.dict {{ ARGS }}

fuzz-prep:
    cd hotmeal/fuzz/browser-wasm
    wasm-pack build --target web --out-dir ../browser-bundle/dist

# List all fuzz targets
list:
    cd hotmeal/fuzz && cargo fuzz list
