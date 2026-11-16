use ratatui::{
	Terminal,
	backend::TestBackend,
	crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

use super::ui::State;
use crate::beld::commands::catalog::{
	Catalog,
	tests::{catalog, image},
};
use crate::types::Formats;

#[test]
fn typing_a_filter_shows_only_the_matching_resources() {
	let mut catalog = catalog(Vec::new());
	let mut state = State::new(&mut catalog);
	let mut terminal = Terminal::new(TestBackend::new(140, 16)).unwrap();

	for character in "/hero".chars() {
		state.handle(&mut catalog, KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
	}
	state.handle(&mut catalog, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
	terminal.draw(|frame| state.draw(frame, &catalog)).unwrap();

	let screen = terminal.backend().to_string();
	assert!(screen.contains("filter> hero"), "{screen}");
	assert!(screen.contains("hero.glb#images/0"), "{screen}");
	assert!(screen.contains("hero.glb#images/1"), "{screen}");
	assert!(!screen.contains("crate.glb"), "{screen}");
	assert!(!screen.contains("music.wav"), "{screen}");
}

fn press(state: &mut State, catalog: &mut Catalog, keys: &[KeyCode]) {
	for &key in keys {
		state.handle(catalog, KeyEvent::new(key, KeyModifiers::NONE));
	}
}

fn screen(state: &mut State, catalog: &Catalog) -> String {
	let mut terminal = Terminal::new(TestBackend::new(140, 16)).unwrap();
	terminal.draw(|frame| state.draw(frame, catalog)).unwrap();
	terminal.backend().to_string()
}

#[test]
fn the_view_still_draws_after_the_selected_line_is_filtered_away_or_reloaded_away() {
	let mut catalog = catalog(Vec::new());
	let mut state = State::new(&mut catalog);

	// Group by class and select the last group, then filter that group away.
	press(
		&mut state,
		&mut catalog,
		&[KeyCode::Char('g'), KeyCode::Down, KeyCode::Down, KeyCode::Enter, KeyCode::End],
	);
	press(
		&mut state,
		&mut catalog,
		&"/hero".chars().map(KeyCode::Char).collect::<Vec<_>>(),
	);
	let filtered = screen(&mut state, &catalog);
	assert!(filtered.contains("▸ Image"), "{filtered}");
	assert!(!filtered.contains("Audio"), "{filtered}");

	// Select the last resource, then read a store that has fewer resources.
	press(
		&mut state,
		&mut catalog,
		&[
			KeyCode::Enter,
			KeyCode::Char('g'),
			KeyCode::Home,
			KeyCode::Enter,
			KeyCode::End,
		],
	);
	let smaller = Catalog::new(
		vec![image("hero.glb#images/0", Formats::RGBA8, 1, 4096, 4096)],
		None,
		Vec::new(),
		&[],
		crate::resource::StorageUsage::default(),
		0,
	);
	state.reload(&mut catalog, smaller);
	let reloaded = screen(&mut state, &catalog);
	assert!(reloaded.contains("filter> hero"), "{reloaded}");
	assert!(reloaded.contains("hero.glb#images/0"), "{reloaded}");
	assert!(!reloaded.contains("hero.glb#images/1"), "{reloaded}");
}
