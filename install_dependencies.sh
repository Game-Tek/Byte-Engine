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

install_packages() {
    if [ "$#" -eq 0 ]; then
        return
    fi

    echo "Installing dependencies: $*"
    if [ "$PKG_MANAGER" == "apt" ]; then
        sudo apt-get install -y "$@" || error_exit "Failed to install dependencies. The most likely cause is an unavailable apt package or network error."
    elif [ "$PKG_MANAGER" == "yum" ]; then
        sudo yum install -y "$@" || error_exit "Failed to install dependencies. The most likely cause is an unavailable yum package or network error."
    elif [ "$PKG_MANAGER" == "dnf" ]; then
        sudo dnf install -y "$@" || error_exit "Failed to install dependencies. The most likely cause is an unavailable dnf package or network error."
    elif [ "$PKG_MANAGER" == "brew" ]; then
        brew install "$@" || error_exit "Failed to install dependencies. The most likely cause is an unavailable Homebrew package or network error."
    fi
}

configure_repositories() {
    if [ "$PKG_MANAGER" != "apt" ]; then
        return
    fi

    if [ -r /etc/os-release ]; then
        . /etc/os-release
    fi

    # CI uses Ubuntu and needs the same Mesa repository the old workflow added.
    # Debian-based devcontainers skip this Ubuntu-only PPA.
    if [ "${ID:-}" == "ubuntu" ]; then
        if ! command_exists add-apt-repository; then
            sudo apt-get update -y || error_exit "Failed to update apt before installing repository tooling. The most likely cause is a network or repository configuration error."
            sudo apt-get install -y software-properties-common || error_exit "Failed to install repository tooling. The most likely cause is an unavailable apt package or network error."
        fi

        sudo add-apt-repository -y ppa:kisak/kisak-mesa || error_exit "Failed to add the Mesa apt repository. The most likely cause is an unavailable Ubuntu PPA or network error."
    fi
}

# Detect OS
OS="unknown"
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
    if command_exists apt-get; then
        PKG_MANAGER="apt"
    elif command_exists dnf; then
        PKG_MANAGER="dnf"
    elif command_exists yum; then
        PKG_MANAGER="yum"
    else
        error_exit "No supported package manager found (apt, dnf, yum). The most likely cause is running on an unsupported Linux distribution."
    fi
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
    if ! command_exists brew; then
        echo "Homebrew not found. Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    fi
    PKG_MANAGER="brew"
else
    error_exit "Unsupported OS: $OSTYPE. The most likely cause is running this script outside Linux or macOS."
fi

configure_repositories

# Update package manager
echo "Updating package manager..."
if [ "$PKG_MANAGER" == "apt" ]; then
    sudo apt-get update -y || error_exit "Failed to update apt. The most likely cause is a network or repository configuration error."
elif [ "$PKG_MANAGER" == "yum" ]; then
    sudo yum update -y || error_exit "Failed to update yum. The most likely cause is a network or repository configuration error."
elif [ "$PKG_MANAGER" == "dnf" ]; then
    sudo dnf update -y || error_exit "Failed to update dnf. The most likely cause is a network or repository configuration error."
elif [ "$PKG_MANAGER" == "brew" ]; then
    brew update || error_exit "Failed to update Homebrew. The most likely cause is a network or repository configuration error."
fi
success "Package manager updated."

# Keep this list in sync with README requirements and Linux CI needs:
# Vulkan development/linker files, Wayland/X11 development packages, ALSA,
# CMake, pkg-config, udev, Mesa GL, Vulkan loader/runtime tools, and validation layers.
if [ "$PKG_MANAGER" == "apt" ]; then
    DEPENDENCIES=(
        cmake
        pkg-config
        libasound2-dev
        libx11-dev
        libx11-xcb-dev
        libxcb1-dev
        libxrandr-dev
        libxi-dev
        libgl1-mesa-dev
        libudev-dev
        libxkbcommon-dev
        libwayland-dev
        libvulkan-dev
        mesa-vulkan-drivers
        libvulkan1
        vulkan-tools
        vulkan-validationlayers
    )
elif [ "$PKG_MANAGER" == "dnf" ] || [ "$PKG_MANAGER" == "yum" ]; then
    DEPENDENCIES=(
        cmake
        pkgconf-pkg-config
        alsa-lib-devel
        libX11-devel
        libxcb-devel
        libXrandr-devel
        libXi-devel
        mesa-libGL-devel
        systemd-devel
        libxkbcommon-devel
        wayland-devel
        vulkan-loader-devel
        mesa-vulkan-drivers
        vulkan-tools
        vulkan-validation-layers
    )
elif [ "$PKG_MANAGER" == "brew" ]; then
    DEPENDENCIES=(
        cmake
        pkg-config
    )
fi

install_packages "${DEPENDENCIES[@]}"
success "All dependencies installed successfully!"
