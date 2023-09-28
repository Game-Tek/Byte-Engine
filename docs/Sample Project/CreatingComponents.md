---
order: -1
icon: container
---

# Creating Components
---

## Weapon
We are going to create our weapon component.

```rust
#[derive(component_derive::Component)]
pub struct Weapon {
	pub resource_id: &'static str,
	#[field] pub transform: maths_rs::Mat4f,
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