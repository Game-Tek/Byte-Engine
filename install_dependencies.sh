#!/bin/bash

# Exit on any error
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# Function to print error and exit
error_exit() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

# Function to print success message
success() {
    echo -e "${GREEN}$1${NC}"
}

# Check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Detect OS
OS="unknown"
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
    if command_exists apt-get; then
        PKG_MANAGER="apt"
    elif command_exists yum; then
        PKG_MANAGER="yum"
    elif command_exists dnf; then
        PKG_MANAGER="dnf"
    else
        error_exit "No supported package manager found (apt, yum, dnf)."
    fi
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
    if ! command_exists brew; then
        echo "Homebrew not found. Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    fi
    PKG_MANAGER="brew"
else
    error_exit "Unsupported OS: $OSTYPE"
fi

# Update package manager
echo "Updating package manager..."
if [ "$PKG_MANAGER" == "apt" ]; then
    sudo apt-get update -y || error_exit "Failed to update apt."
elif [ "$PKG_MANAGER" == "yum" ]; then
    sudo yum update -y || error_exit "Failed to update yum."
elif [ "$PKG_MANAGER" == "dnf" ]; then
    sudo dnf update -y || error_exit "Failed to update dnf."
elif [ "$PKG_MANAGER" == "brew" ]; then
    brew update || error_exit "Failed to update Homebrew."
fi
success "Package manager updated."

# List of dependencies to install
DEPENDENCIES=("cmake" "libwayland-dev" "libasound2-dev" "libx11-xcb-dev" "libvulkan-dev" "vulkan-tools" "vulkan-validationlayers")

# Check and install dependencies
for dep in "${DEPENDENCIES[@]}"; do
    if ! command_exists "$dep"; then
        echo "Installing $dep..."
        if [ "$PKG_MANAGER" == "apt" ]; then
            sudo apt-get install -y "$dep" || error_exit "Failed to install $dep."
        elif [ "$PKG_MANAGER" == "yum" ]; then
            sudo yum install -y "$dep" || error_exit "Failed to install $dep."
        elif [ "$PKG_MANAGER" == "dnf" ]; then
            sudo dnf install -y "$dep" || error_exit "Failed to install $dep."
        elif [ "$PKG_MANAGER" == "brew" ]; then
            brew install "$dep" || error_exit "Failed to install $dep."
        fi
        success "$dep installed."
    else
        success "$dep is already installed."
    fi
done

success "All dependencies installed successfully!"