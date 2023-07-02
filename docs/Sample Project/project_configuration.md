---
order: 0
---

# Project Setup

---

Our application needs a place to start.

Create a file called `main.rs`.
```bash
touch src/main.rs
```

Then add the following code:

```rust
use byte_engine;
fn main() {
	let mut application = byte_engine::Application::new("Gallery Shooter");
}
```

This will create a new application with the name "Gallery Shooter". The application manages all utilities we will need to run our game.

Let's think about what we need to make our game.

We need a player, targets, weapon.

So let's define them.

```rust
use byte_engine;
struct Weapon {
	// ...
}
```

```rust
use byte_engine;
fn main() {
	// ...
	let weapon_handle = Weapon::new(&mut application);
}
```

We a need a place to put them.

```rust
use byte_engine;
fn main() {
	// ...
	let mut level = byte_engine::Level::new(&mut application);
}
```