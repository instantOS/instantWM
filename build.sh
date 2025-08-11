#!/usr/bin/env bash
# InstantWM Rust Build Script
# This script helps build and install the Rust version of InstantWM

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if we're in the right directory
if [[ ! -f "Cargo.toml" ]]; then
    print_error "Cargo.toml not found. Please run this script from the instantWM directory."
    exit 1
fi

# Function to check dependencies
check_dependencies() {
    print_status "Checking dependencies..."

    # Check for Rust
    if ! command -v rustc &> /dev/null; then
        print_error "Rust is not installed. Please install Rust from https://rustup.rs/"
        exit 1
    fi

    # Check for Cargo
    if ! command -v cargo &> /dev/null; then
        print_error "Cargo is not found. Please install Rust with Cargo."
        exit 1
    fi

    # Check Rust version (need at least 1.70 for Smithay)
    RUST_VERSION=$(rustc --version | cut -d' ' -f2)
    print_status "Found Rust version: $RUST_VERSION"

    # Check for pkg-config
    if ! command -v pkg-config &> /dev/null; then
        print_warning "pkg-config not found. Some dependencies might fail to compile."
        print_warning "On Ubuntu/Debian: sudo apt install pkg-config"
        print_warning "On Arch: sudo pacman -S pkgconf"
    fi

    print_success "Dependencies check completed"
}

# Function to build the project
build_project() {
    local build_type="${1:-debug}"

    print_status "Building InstantWM in $build_type mode..."

    if [[ "$build_type" == "release" ]]; then
        cargo build --release
    else
        cargo build
    fi

    if [[ $? -eq 0 ]]; then
        print_success "Build completed successfully!"
    else
        print_error "Build failed!"
        exit 1
    fi
}

# Function to run tests
run_tests() {
    print_status "Running tests..."
    cargo test

    if [[ $? -eq 0 ]]; then
        print_success "All tests passed!"
    else
        print_error "Some tests failed!"
        exit 1
    fi
}

# Function to check code quality
check_quality() {
    print_status "Checking code formatting..."
    cargo fmt --check

    if [[ $? -ne 0 ]]; then
        print_warning "Code needs formatting. Run 'cargo fmt' to fix."
    else
        print_success "Code is properly formatted."
    fi

    print_status "Running clippy checks..."
    cargo clippy -- -D warnings

    if [[ $? -eq 0 ]]; then
        print_success "No clippy warnings found!"
    else
        print_error "Clippy found issues!"
        exit 1
    fi
}

# Function to install the binaries
install_binaries() {
    print_status "Installing binaries..."

    # Check if we have a release build
    if [[ ! -f "target/release/instantwm" ]]; then
        print_error "Release build not found. Run './build.sh --release' first."
        exit 1
    fi

    # Install main binary
    sudo install -m 755 target/release/instantwm /usr/local/bin/
    print_success "Installed instantwm to /usr/local/bin/"

    # Install CLI tool
    if [[ -f "target/release/instantctl" ]]; then
        sudo install -m 755 target/release/instantctl /usr/local/bin/
        print_success "Installed instantctl to /usr/local/bin/"
    fi

    # Install desktop file if it exists
    if [[ -f "instantwm.desktop" ]]; then
        sudo install -m 644 instantwm.desktop /usr/share/wayland-sessions/
        print_success "Installed desktop file to /usr/share/wayland-sessions/"
    fi

    print_success "Installation completed!"
}

# Function to setup configuration
setup_config() {
    print_status "Setting up configuration..."

    local config_dir="$HOME/.config/instantwm"
    local config_file="$config_dir/config.toml"

    # Create config directory
    mkdir -p "$config_dir"

    # Copy example config if user doesn't have one
    if [[ ! -f "$config_file" ]]; then
        cp config.toml.example "$config_file"
        print_success "Copied example configuration to $config_file"
        print_status "Edit $config_file to customize your settings"
    else
        print_warning "Configuration file already exists at $config_file"
        print_status "To reset: rm $config_file && ./build.sh --setup-config"
    fi
}

# Function to show usage
show_usage() {
    echo "InstantWM Build Script"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "OPTIONS:"
    echo "  --help          Show this help message"
    echo "  --check         Check dependencies only"
    echo "  --build         Build in debug mode (default)"
    echo "  --release       Build in release mode"
    echo "  --test          Run tests"
    echo "  --quality       Check code quality (format + clippy)"
    echo "  --install       Install binaries (requires release build)"
    echo "  --setup-config  Setup user configuration"
    echo "  --all           Run check, quality, test, and release build"
    echo ""
    echo "Examples:"
    echo "  $0                    # Build in debug mode"
    echo "  $0 --release         # Build optimized release"
    echo "  $0 --all             # Full build pipeline"
    echo "  $0 --release --install  # Build and install"
}

# Main script logic
main() {
    local should_check_deps=false
    local should_build=false
    local should_test=false
    local should_check_quality=false
    local should_install=false
    local should_setup_config=false
    local build_type="debug"

    # Parse command line arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --help|-h)
                show_usage
                exit 0
                ;;
            --check)
                should_check_deps=true
                shift
                ;;
            --build)
                should_build=true
                shift
                ;;
            --release)
                should_build=true
                build_type="release"
                shift
                ;;
            --test)
                should_test=true
                shift
                ;;
            --quality)
                should_check_quality=true
                shift
                ;;
            --install)
                should_install=true
                shift
                ;;
            --setup-config)
                should_setup_config=true
                shift
                ;;
            --all)
                should_check_deps=true
                should_check_quality=true
                should_test=true
                should_build=true
                build_type="release"
                shift
                ;;
            *)
                print_error "Unknown option: $1"
                show_usage
                exit 1
                ;;
        esac
    done

    # Default action if no arguments
    if [[ $# -eq 0 ]] && [[ "$should_check_deps" == false ]] && [[ "$should_build" == false ]] && [[ "$should_test" == false ]] && [[ "$should_check_quality" == false ]] && [[ "$should_install" == false ]] && [[ "$should_setup_config" == false ]]; then
        should_build=true
    fi

    # Show header
    echo -e "${BLUE}======================================${NC}"
    echo -e "${BLUE}    InstantWM Rust Build Script      ${NC}"
    echo -e "${BLUE}======================================${NC}"
    echo ""

    # Execute requested actions
    if [[ "$should_check_deps" == true ]] || [[ "$should_build" == true ]] || [[ "$should_test" == true ]]; then
        check_dependencies
        echo ""
    fi

    if [[ "$should_check_quality" == true ]]; then
        check_quality
        echo ""
    fi

    if [[ "$should_test" == true ]]; then
        run_tests
        echo ""
    fi

    if [[ "$should_build" == true ]]; then
        build_project "$build_type"
        echo ""
    fi

    if [[ "$should_install" == true ]]; then
        install_binaries
        echo ""
    fi

    if [[ "$should_setup_config" == true ]]; then
        setup_config
        echo ""
    fi

    print_success "All operations completed successfully!"

    # Show next steps
    if [[ "$should_build" == true ]] && [[ "$should_install" == false ]]; then
        echo ""
        echo -e "${BLUE}Next steps:${NC}"
        echo "  1. Install: ./build.sh --install"
        echo "  2. Setup config: ./build.sh --setup-config"
        echo "  3. Run: instantwm"
    fi
}

# Run main function with all arguments
main "$@"
