# Contributing to instantWM

Thank you for your interest in contributing to instantWM! This document provides guidelines for contributing to the project.

## Development Setup

1. **Prerequisites**:
   - Rust 1.70 or later
   - Wayland development libraries
   - Smithay dependencies

2. **Clone and build**:
   ```bash
   git clone https://github.com/instantos/instantwm-rs
   cd instantwm-rs
   make build
   ```

## Code Style

- Follow Rust's standard formatting (`cargo fmt`)
- Use `cargo clippy` to catch common issues
- Write clear, concise commit messages
- Include tests for new functionality

## Architecture

The project is organized into several key modules:

- `compositor`: Core Wayland compositor using Smithay
- `window_manager`: Window management logic
- `input`: Input handling (keyboard/mouse)
- `config`: Configuration management
- `cli`: Command-line interface

## Testing

```bash
# Run all tests
make test

# Run with debug logging
make run

# Check code style
make check
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Run `make check` to ensure code quality
6. Submit a pull request

## Reporting Issues

When reporting issues, please include:
- Operating system and version
- Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs actual behavior

## Questions?

Feel free to open an issue for questions or discussion.