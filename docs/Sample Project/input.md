If our game is going to be any good we probably want our player to be able to move.

```rust
use byte_engine;
fn main() {
	// ...
	let lookaround = byte_engine::input::Action::new("Lookaround", byte_engine::input::ActionType::Vector2, &[byte_engine::input::Mouse]); // Declare a new action called "Lookaround" that is of type vector2 and is bound to the mouse movement

	let mut player = byte_engine::Entity::new(&mut level); // Create a new entity

	application.tie(&player, Player::orientation, &lookaround, Action::value); // Tie the orientation of the player to the value of the lookaround action
}
```