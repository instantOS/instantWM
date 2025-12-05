# instantWM tasks

# Format all C source and header files
fmt:
    find . -name "*.c" -o -name "*.h" | xargs clang-format -i

# Build the project
build:
    make

# Clean build artifacts
clean:
    make clean

# Install the project
install:
    make install

# Uninstall the project
uninstall:
    make uninstall

# Create distribution package
dist:
    make dist
