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
