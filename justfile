python_release_script := ".github/scripts/prepare-release.py"

install:
    bash ./scripts/install.sh

check:
    uvx ty check {{python_release_script}}
    uvx ruff check {{python_release_script}}
    uvx ruff format --check {{python_release_script}}

fmt:
    uvx ruff check --fix {{python_release_script}}
    uvx ruff format {{python_release_script}}
    cargo clippy --fix --allow-dirty
    cargo fmt

# Build an optimized but fully symbolized binary. This is not a release build.
profile-build:
    RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C force-frame-pointers=yes" cargo build --profile profiling --bin instantwm --bin instantwmctl

# Run from an active TTY for the DRM backend. workload is "standard" or "manual".
profile duration="20" workload="standard": profile-build
    bash scripts/profile-capture.sh "{{duration}}" "{{workload}}"

# Recreate text/JSON reports from an existing capture directory or perf.data.
profile-report capture="target/profiles/latest":
    python3 scripts/profile-report.py "{{capture}}"

# Temporarily allow unprivileged per-process, user-space profiling.
profile-permissions:
    sudo sysctl -w kernel.perf_event_paranoid=2

# Optional visual exploration. The text/JSON reports do not require Samply.
profile-samply capture="target/profiles/latest":
    bash scripts/profile-samply.sh "{{capture}}"
