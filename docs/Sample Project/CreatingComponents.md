---
order: -1
icon: container
---

# Creating Components
---

## Weapon
We are going to create our weapon component.

```rust
use byte_engine;
struct Weapon {
	mesh: ComponentHandle<Mesh>,
}

impl Weapon {
	fn new(application: &mut Application) -> ComponentHandle<Self> {
		let mesh = Mesh::new(application, "weapon.obj");
		let weapon = Self {
			mesh,
		};
		application.add_component(weapon)
	}

	fn fire(&self) {
		// TODO: firing logic
	}
}
```