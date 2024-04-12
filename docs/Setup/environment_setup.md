---
icon: tools
---

# Environment Setup

---

Byte depends on a few external tools to work properly. You will need to install them before you can start using Byte.

## Required

### Linux packages

```bash
sudo apt install -y libwayland-dev libasound2-dev libvulkan1 vulkan-tools vulkan-validationlayers vulkan-sdk
```

#### Rust(up)
Byte is written in Rust. You will need to install Rust to be able to compile and run Byte.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

---

## Optional

### Mold
Mold is fast linker. It is not required to run Byte, but if you are writing native code, it is recommended.

```bash
sudo apt install mold
```

### RenderDoc
RenderDoc is a graphics debugger. It is not required to run Byte, but it is useful for debugging.
Byte has an integration with RenderDoc to facilitate debugging.


```bash
sudo apt install renderdoc
```