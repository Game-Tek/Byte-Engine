//! Browse a resource store interactively in the terminal: filter, group, and sort every resource, and follow its
//! sources and references.
//!
//! The behavior is specified in the BELD command-line reference, under Browse.

#[cfg(test)]
mod tests;
mod ui;

use std::io::IsTerminal as _;

use ratatui::{
	DefaultTerminal,
	crossterm::event::{self, Event, KeyEventKind},
};
use utils::Glob;

use self::ui::{Exit, State};
use super::{
	catalog::{self, Catalog},
	shared::log_failure,
};

/// Opens the browser on the resource store at `destination_path`, checking resources against the sources at
/// `source_path`.
///
/// `roots` are the project's `resources.roots` glob patterns, which decide what `is:unreachable` shows.
/// The store is read once; `R` reads it again. The command never modifies the store.
pub async fn browse(source_path: String, destination_path: String, roots: Vec<Glob>) -> Result<(), i32> {
	if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
		log::error!(
			"`beld browse` needs an interactive terminal. The most likely cause is that its input or output is redirected to a pipe or a file. Run it directly in a terminal, or use `beld query --format json` in scripts."
		);
		return Err(1);
	}

	let mut catalog = catalog::load(&source_path, &destination_path, &roots)
		.await
		.map_err(log_failure)?;
	let mut state = State::new(&mut catalog);

	// Log lines written while the browser owns the screen would break its layout.
	let log_level = log::max_level();
	log::set_max_level(log::LevelFilter::Off);
	let mut terminal = ratatui::init();
	let result = run(
		&mut terminal,
		&mut catalog,
		&mut state,
		&source_path,
		&destination_path,
		&roots,
	)
	.await;
	ratatui::restore();
	log::set_max_level(log_level);

	result.map_err(|error| {
		log::error!(
			"The resource browser stopped because the terminal failed. The most likely cause is that the terminal closed or stopped accepting output. Run `beld browse` again in an open terminal. Error: {error}"
		);
		1
	})
}

/// Draws the browser and handles key presses until the user quits.
async fn run(
	terminal: &mut DefaultTerminal,
	catalog: &mut Catalog,
	state: &mut State,
	source_path: &str,
	destination_path: &str,
	roots: &[Glob],
) -> std::io::Result<()> {
	loop {
		terminal.draw(|frame| state.draw(frame, catalog))?;
		// Only a key press or a resize changes what the browser shows, so other events don't redraw it.
		let key = loop {
			match event::read()? {
				Event::Key(key) if key.kind == KeyEventKind::Press => break Some(key),
				Event::Resize(..) => break None,
				_ => {}
			}
		};
		let Some(key) = key else {
			continue;
		};
		match state.handle(catalog, key) {
			None => {}
			Some(Exit::Quit) => return Ok(()),
			Some(Exit::Reload) => {
				state.notify("Reading the resource store again…".to_string());
				terminal.draw(|frame| state.draw(frame, catalog))?;
				match catalog::load(source_path, destination_path, roots).await {
					Ok(reloaded) => state.reload(catalog, reloaded),
					Err(error) => state.notify(error),
				}
			}
		}
	}
}
