---
icon: tools
---

# Environment Setup

---

Byte depends on a few external tools to work properly. You will need to install them before you can start using Byte.

## Required

### Linux packages

```bash
sudo apt install -y libwayland-dev libasound2-dev libx11-xcb-dev libvulkan-dev vulkan-tools vulkan-validationlayers
```

#### Rust(up)
Byte is written in Rust. You will need to install Rust to be able to compile and run Byte.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

---

## Optional

### Mold
[Mold](https://github.com/rui314/mold) is fast linker. It is not required by Byte, but we recommend for it's improved iteration speed.

Install steps, including Rust setup, are outlined in it's repository.

### RenderDoc
RenderDoc is a graphics debugger. It is not required to run Byte, but it is useful for debugging during graphics development.
Byte has an integration with RenderDoc to facilitate debugging.


```bash
sudo apt install renderdoc
```
