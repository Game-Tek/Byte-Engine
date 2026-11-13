//! The terminal console: a full-screen view of the application's log with a command line under it.
//!
//! The console is a second transport over the [`DefaultInspector`], beside the
//! [`HttpInspectorServer`](super::http::HttpInspectorServer). [`GraphicsApplication`] starts it with [`spawn`]. Its
//! first command, `set`, changes a runtime setting through [`DefaultInspector::update_configuration`].
//! See the [console reference](/docs/reference/console) for the screen, the keys, and the commands.
//!
//! [`GraphicsApplication`]: crate::application::graphics::GraphicsApplication

use std::{
	collections::VecDeque,
	io::{self, IsTerminal as _, Write as _},
	time::{Duration, Instant},
};

use ratatui::{
	DefaultTerminal, Frame,
	crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
	layout::{Constraint, Layout, Position, Rect},
	style::{Color, Style, Stylize as _},
	text::{Line, Span},
	widgets::{Block, Paragraph},
};

use crate::{
	application::{
		Events, LoopWaker,
		log_capture::{self, Entry, LineKind},
		parameters::Parameters,
		thread::Thread,
	},
	configuration::{ConfigurationEventId, ConfigurationUpdateState, ConfigurationValue},
	core::{EntityHandle, listener::DefaultListener, listener::Listener as _},
	inspector::DefaultInspector,
};

/// How long `set` waits for the owning system before it reports the update as pending.
const RESULT_WAIT: Duration = Duration::from_secs(1);

/// How long the console waits for a key before it looks for new log lines, results, and the application closing.
const POLL: Duration = Duration::from_millis(50);

const HELP: &str = "\
set <parameter> <value>  Changes a runtime setting, such as `set render.lod.force 2`.
help                     Lists the commands.";

const KEYS: &str = "↑↓ history · PgUp/PgDn log · Ctrl+C quit";

/// Starts the console on an application thread when the `enabled` parameter isn't `false` and both standard input and
/// standard output are terminals. Otherwise it returns `None` and never touches the terminal, so runs in pipelines,
/// test harnesses, and agents keep their standard input and output.
///
/// The console ends when `events` delivers [`Events::Close`]; join the returned thread on shutdown so the terminal is
/// restored before the process exits.
///
/// # Parameters
/// Read from `parameters`, the `console` subtree of the application's:
/// - `enabled`: Starts the console (`true`) or never starts it (`false`). Set it to `false` when the application reads
///   standard input or writes standard output itself. The default is `true`.
///
/// Next, type `help` in the terminal, or see the [console reference](/docs/reference/console).
pub fn spawn(
	inspector: EntityHandle<DefaultInspector>,
	waker: LoopWaker,
	events: DefaultListener<Events>,
	parameters: impl Parameters,
) -> Option<Thread> {
	let enabled = parameters
		.get_parameter("enabled")
		.is_none_or(|parameter| parameter.as_bool_simple());
	if !enabled || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
		return None;
	}

	Some(Thread::new(events, move |mut events| {
		let mut terminal = match ratatui::try_init() {
			Ok(terminal) => terminal,
			Err(error) => {
				log::error!(
					"Console did not start. The most likely cause is that the terminal refused raw mode. Set `console.enabled` to `false` to run without it: {error}"
				);
				return;
			}
		};
		log_capture::begin();
		let result = run(&mut terminal, &inspector, &waker, &mut events);
		ratatui::restore();

		// The screen the log was drawn on is gone; write the log where it would have gone without the console, so it
		// stays in the terminal's history.
		let mut stderr = io::stderr().lock();
		for entry in log_capture::end() {
			let _ = writeln!(stderr, "{}", entry.text);
		}
		drop(stderr);

		if let Err(error) = result {
			log::error!(
				"Console stopped because the terminal failed. The most likely cause is that the terminal closed or stopped accepting output. Restart the application from an open terminal to use the console again: {error}"
			);
		}
	}))
}

/// Draws the console and handles keys until the application closes.
fn run(
	terminal: &mut DefaultTerminal,
	inspector: &DefaultInspector,
	waker: &LoopWaker,
	events: &mut DefaultListener<Events>,
) -> io::Result<()> {
	let mut console = Console::default();
	let mut drawn = None;
	loop {
		while let Some(event) = events.read() {
			if event == Events::Close {
				return Ok(());
			}
		}
		console.collect_results(inspector, Instant::now());
		// Redraw only when a line arrived, a key or resize changed the screen, or a result came in.
		let revision = (log_capture::revision(), console.revision);
		if drawn != Some(revision) {
			terminal.draw(|frame| log_capture::with_entries(|entries| console.draw(frame, entries)))?;
			drawn = Some(revision);
		}
		if !event::poll(POLL)? {
			continue;
		}
		match event::read()? {
			Event::Key(key) if key.kind == KeyEventKind::Press => match console.handle(key) {
				Some(Action::Run(line)) => console.run_line(&line, inspector, waker, Instant::now()),
				Some(Action::Quit) => {
					inspector.close_application();
					waker.wake();
				}
				None => {}
			},
			Event::Resize(..) => console.revision += 1,
			_ => {}
		}
	}
}

/// The `Action` enum is what a key asks of the console loop.
#[derive(Debug, PartialEq, Eq)]
enum Action {
	/// Run this command line.
	Run(String),
	/// Close the application.
	Quit,
}

/// The `Awaiting` struct is one `set` whose owning system hasn't answered yet.
struct Awaiting {
	id: ConfigurationEventId,
	parameter: String,
	since: Instant,
	/// Whether the console already said the update is pending.
	reported_pending: bool,
}

/// The `Console` struct is what the console shows and edits, apart from the terminal, so keys and commands can be
/// tested without one.
#[derive(Default)]
struct Console {
	/// The command line being typed.
	line: String,
	/// The cursor's position in [`Self::line`], in characters.
	cursor: usize,
	/// Commands run so far, oldest first.
	history: Vec<String>,
	/// The [`Self::history`] entry shown in the command line, while browsing it.
	recalled: Option<usize>,
	/// How many wrapped rows the log pane is scrolled up from its newest row.
	scroll: usize,
	/// The log pane's height at the last draw, which Page Up and Page Down move by.
	page: usize,
	/// The latest result.
	status: String,
	awaiting: Vec<Awaiting>,
	/// Counts changes to what the console shows apart from the log lines.
	revision: u64,
}

impl Console {
	/// Applies one key press to the command line, the history, or the log pane, and returns what the loop must do.
	fn handle(&mut self, key: KeyEvent) -> Option<Action> {
		self.revision += 1;
		if key.modifiers.contains(KeyModifiers::CONTROL) {
			return (key.code == KeyCode::Char('c')).then_some(Action::Quit);
		}
		match key.code {
			KeyCode::Char(character) => {
				let at = self.byte_index(self.cursor);
				self.line.insert(at, character);
				self.cursor += 1;
			}
			KeyCode::Backspace if self.cursor > 0 => {
				self.cursor -= 1;
				self.line.remove(self.byte_index(self.cursor));
			}
			KeyCode::Delete if self.cursor < self.line.chars().count() => {
				self.line.remove(self.byte_index(self.cursor));
			}
			KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
			KeyCode::Right => self.cursor = (self.cursor + 1).min(self.line.chars().count()),
			KeyCode::Home => self.cursor = 0,
			KeyCode::End => self.cursor = self.line.chars().count(),
			KeyCode::Esc => self.replace_line(String::new()),
			KeyCode::Up => {
				let recalled = match self.recalled {
					None => self.history.len().checked_sub(1),
					Some(index) => Some(index.saturating_sub(1)),
				};
				if let Some(index) = recalled {
					self.recalled = Some(index);
					self.replace_line(self.history[index].clone());
				}
			}
			KeyCode::Down => {
				if let Some(index) = self.recalled {
					let next = index + 1;
					self.recalled = (next < self.history.len()).then_some(next);
					self.replace_line(self.history.get(next).cloned().unwrap_or_default());
				}
			}
			KeyCode::PageUp => self.scroll += self.page.max(1),
			KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(self.page.max(1)),
			KeyCode::Enter => {
				let line = std::mem::take(&mut self.line);
				self.cursor = 0;
				self.recalled = None;
				// The newest rows show the command's result, so running a command scrolls back to them.
				self.scroll = 0;
				return Some(Action::Run(line));
			}
			_ => {}
		}
		None
	}

	/// Runs one command line and reports its result, or queues a `set` whose result [`Self::collect_results`] reports.
	fn run_line(&mut self, line: &str, inspector: &DefaultInspector, waker: &LoopWaker, now: Instant) {
		let line = line.trim();
		if line.is_empty() {
			return;
		}
		if self.history.last().is_none_or(|last| last != line) {
			self.history.push(line.to_string());
		}
		log_capture::push(LineKind::Console, format!("> {line}"));
		match parse_line(line) {
			Ok(Command::Help) => self.report(HELP.to_string()),
			Ok(Command::Set { parameter, value }) => {
				let id = inspector.update_configuration(parameter, value).id();
				// An idle application waits for input; the update is input, so it runs a tick that applies it.
				waker.wake();
				self.awaiting.push(Awaiting {
					id,
					parameter: parameter.to_string(),
					since: now,
					reported_pending: false,
				});
				self.collect_results(inspector, now);
			}
			Err(error) => self.report(error),
		}
	}

	/// Reports every awaited `set` the owning system answered, and says once that a `set` is pending when its system
	/// hasn't answered within [`RESULT_WAIT`].
	fn collect_results(&mut self, inspector: &DefaultInspector, now: Instant) {
		let mut reports = Vec::new();
		self.awaiting.retain_mut(|awaiting| {
			let state = inspector.configuration.event(awaiting.id).map(|event| event.state().clone());
			let parameter = &awaiting.parameter;
			match state {
				Some(ConfigurationUpdateState::Set { value }) => reports.push(format!("{parameter} set to {}", show_value(&value))),
				Some(ConfigurationUpdateState::NotSet { reason }) => reports.push(format!("{parameter} not set: {reason}")),
				_ => {
					if !awaiting.reported_pending && now.duration_since(awaiting.since) >= RESULT_WAIT {
						awaiting.reported_pending = true;
						reports.push(format!(
							"{parameter} is pending. Its owning system did not take it within a second, most likely because what it names, such as a render pass, isn't installed yet. It applies once the system takes it."
						));
					}
					return true;
				}
			}
			false
		});
		for report in reports {
			self.report(report);
		}
	}

	/// Shows `text` in the status line and keeps it in the log pane.
	fn report(&mut self, text: String) {
		// The status line has one row: it shows the first line of a report, and the log keeps all of it.
		self.status = text.lines().next().unwrap_or_default().to_string();
		log_capture::push(LineKind::Console, text);
		self.revision += 1;
	}

	/// Draws the log pane, the command line, and the status line.
	fn draw(&mut self, frame: &mut Frame, entries: &VecDeque<Entry>) {
		let [log_area, command_area, status_area] =
			Layout::vertical([Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)]).areas(frame.area());

		let block = Block::bordered().title(" Log ");
		let inner = block.inner(log_area);
		frame.render_widget(block, log_area);
		self.page = inner.height as usize;
		let rows = self.log_rows(entries, inner);
		frame.render_widget(Paragraph::new(rows), inner);

		// Keep the cursor on screen by showing the end of a line longer than the area.
		let width = (command_area.width as usize).saturating_sub(3);
		let skip = self.cursor.saturating_sub(width);
		let shown: String = self.line.chars().skip(skip).collect();
		frame.render_widget(Paragraph::new(Line::from(format!("> {shown}"))), command_area);
		frame.set_cursor_position(Position::new(
			command_area.x + 2 + (self.cursor - skip) as u16,
			command_area.y,
		));

		let status = if self.status.is_empty() {
			format!("Type `help` to list the commands · {KEYS}")
		} else {
			format!("{} · {KEYS}", self.status)
		};
		frame.render_widget(Paragraph::new(Line::from(status).dim()), status_area);
	}

	/// Returns the rows the log pane shows: the lines wrapped to the pane's width, the newest at the bottom, moved up
	/// by [`Self::scroll`], which this lowers until the oldest row is at the top at most.
	fn log_rows<'a>(&mut self, entries: &'a VecDeque<Entry>, area: Rect) -> Vec<Line<'a>> {
		let width = (area.width as usize).max(1);
		let height = area.height as usize;
		loop {
			let mut rows = scrolled_rows(entries, width, height, self.scroll);
			// Scrolled past the oldest row, the pane would be part empty: scroll down by what's missing.
			if rows.len() < height && self.scroll > 0 {
				self.scroll = self.scroll.saturating_sub(height - rows.len());
				continue;
			}
			rows.reverse();
			return rows;
		}
	}

	/// Replaces the command line and puts the cursor at its end.
	fn replace_line(&mut self, line: String) {
		self.cursor = line.chars().count();
		self.line = line;
	}

	/// Returns the byte offset of character `index` in the command line.
	fn byte_index(&self, index: usize) -> usize {
		self.line.char_indices().nth(index).map_or(self.line.len(), |(at, _)| at)
	}
}

/// Returns up to `height` rows of `entries` wrapped to `width`, newest first, after skipping the newest `scroll` rows.
fn scrolled_rows<'a>(entries: &'a VecDeque<Entry>, width: usize, height: usize, scroll: usize) -> Vec<Line<'a>> {
	let mut rows = Vec::with_capacity(height);
	let mut skipped = 0;
	// Walk from the newest entry up, so a pane at the bottom only wraps the entries it shows.
	for entry in entries.iter().rev() {
		let entry_rows = wrap(&entry.text, width);
		for (index, row) in entry_rows.iter().enumerate().rev() {
			if skipped < scroll {
				skipped += 1;
			} else if rows.len() < height {
				rows.push(style_row(entry.kind, index == 0, row));
			}
		}
		if rows.len() == height {
			break;
		}
	}
	rows
}

/// Splits `text` at its line breaks, then into rows of at most `width` characters.
fn wrap(text: &str, width: usize) -> Vec<&str> {
	let mut rows = Vec::new();
	for line in text.split('\n') {
		let mut rest = line;
		loop {
			let end = rest.char_indices().nth(width).map_or(rest.len(), |(at, _)| at);
			rows.push(&rest[..end]);
			rest = &rest[end..];
			if rest.is_empty() {
				break;
			}
		}
	}
	rows
}

/// Styles one row: a record's level in its level's color, a console line in cyan, and everything else plain.
fn style_row(kind: LineKind, first: bool, row: &str) -> Line<'_> {
	let level = match kind {
		LineKind::Console => return Line::styled(row, Style::new().fg(Color::Cyan)),
		LineKind::Log(level) => level,
	};
	// Only a record's first row holds its level, and a narrow pane may cut the row inside it.
	if !first || row.len() <= log_capture::LEVEL.start {
		return Line::raw(row);
	}
	let style = match level {
		log::Level::Error => Style::new().fg(Color::Red),
		log::Level::Warn => Style::new().fg(Color::Yellow),
		log::Level::Info => Style::new().fg(Color::Green),
		log::Level::Debug | log::Level::Trace => Style::new().dim(),
	};
	let end = row.len().min(log_capture::LEVEL.end);
	Line::from(vec![
		Span::raw(&row[..log_capture::LEVEL.start]),
		Span::styled(&row[log_capture::LEVEL.start..end], style),
		Span::raw(&row[end..]),
	])
}

/// Returns a value as typed, without the quotes JSON would put around text.
fn show_value(value: &ConfigurationValue) -> String {
	match value {
		ConfigurationValue::Bool(value) => value.to_string(),
		ConfigurationValue::Integer(value) => value.to_string(),
		ConfigurationValue::Float(value) => value.to_string(),
		ConfigurationValue::Text(value) => value.clone(),
	}
}

/// The `Command` enum is one parsed command line, borrowing its names from the line.
#[derive(Debug, PartialEq)]
enum Command<'a> {
	/// Changes the runtime setting named `parameter`.
	Set { parameter: &'a str, value: ConfigurationValue },
	/// Lists the commands.
	Help,
}

/// Parses one non-empty, trimmed command line.
fn parse_line(line: &str) -> Result<Command<'_>, String> {
	let (command, arguments) = split_word(line);
	match command {
		"set" => match split_word(arguments) {
			(parameter, value) if !parameter.is_empty() && !value.is_empty() => Ok(Command::Set {
				parameter,
				value: read_value(value),
			}),
			_ => Err(
				"`set` needs a parameter and a value. The most likely cause is that one of them is missing. Type `set <parameter> <value>`, such as `set render.lod.force 2`."
					.to_string(),
			),
		},
		"help" => Ok(Command::Help),
		_ => Err(format!(
			"Console command `{command}` is unknown. The most likely cause is a typo. Type `help` to list the commands."
		)),
	}
}

/// Splits `text` at its first run of spaces into the first word and the rest.
fn split_word(text: &str) -> (&str, &str) {
	match text.split_once(char::is_whitespace) {
		Some((word, rest)) => (word, rest.trim_start()),
		None => (text, ""),
	}
}

/// Reads a value as JSON, as the inspector's configuration route does, so `true`, `2`, and `0.5` keep their types;
/// text that isn't JSON, such as `off`, stays text.
fn read_value(text: &str) -> ConfigurationValue {
	serde_json::from_str(text).unwrap_or_else(|_| ConfigurationValue::Text(text.to_string()))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use ratatui::{Terminal, backend::TestBackend};

	use super::*;
	use crate::{configuration::Configuration, core::message_bus::MessageBus, metrics::Metrics};

	fn inspector(configuration: Configuration) -> DefaultInspector {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		DefaultInspector::new(
			crate::core::channel::DefaultChannel::new(),
			configuration,
			message_bus.new_scope("console-test-world"),
			Arc::new(Metrics::new()),
		)
	}

	fn key(code: KeyCode) -> KeyEvent {
		KeyEvent::new(code, KeyModifiers::NONE)
	}

	/// Types `text` into the console, one key per character.
	fn type_text(console: &mut Console, text: &str) {
		for character in text.chars() {
			assert_eq!(console.handle(key(KeyCode::Char(character))), None);
		}
	}

	fn console_line(text: &str) -> Entry {
		Entry {
			kind: LineKind::Console,
			text: text.to_string(),
		}
	}

	/// Draws `console` over `entries` on a terminal of `width` by `height` cells and returns its buffer.
	fn draw_buffer(console: &mut Console, entries: &VecDeque<Entry>, width: u16, height: u16) -> ratatui::buffer::Buffer {
		let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
		terminal.draw(|frame| console.draw(frame, entries)).expect("draw");
		terminal.backend().buffer().clone()
	}

	/// Draws `console` over `entries` and returns the text of each row.
	fn draw(console: &mut Console, entries: &VecDeque<Entry>, width: u16, height: u16) -> Vec<String> {
		let buffer = draw_buffer(console, entries, width, height);
		(0..height)
			.map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
			.collect()
	}

	#[test]
	fn values_keep_their_json_types_and_other_text_stays_text() {
		let value = |line| match parse_line(line) {
			Ok(Command::Set { value, .. }) => value,
			other => panic!("expected a set command, got {other:?}"),
		};
		assert_eq!(value("set render.lod.force 2"), ConfigurationValue::Integer(2));
		assert_eq!(value("set render.gtao.radius 0.5"), ConfigurationValue::Float(0.5));
		assert_eq!(value("set render.gi.enabled true"), ConfigurationValue::Bool(true));
		assert_eq!(value("set render.lod.force off"), ConfigurationValue::from("off"));
		assert_eq!(
			value("set render.pass.bloom \"bypassed\""),
			ConfigurationValue::from("bypassed")
		);
	}

	#[test]
	fn set_names_the_first_word_and_takes_the_rest_as_the_value() {
		assert_eq!(
			parse_line("set   render.pass.ui   enabled now"),
			Ok(Command::Set {
				parameter: "render.pass.ui",
				value: ConfigurationValue::from("enabled now"),
			})
		);
	}

	#[test]
	fn malformed_commands_are_errors() {
		assert_eq!(parse_line("help"), Ok(Command::Help));
		assert!(parse_line("set render.lod.force").is_err());
		assert!(parse_line("set").is_err());
		assert!(parse_line("bogus a b").unwrap_err().contains("`bogus` is unknown"));
	}

	#[test]
	fn keys_edit_the_command_line_and_enter_runs_it() {
		let mut console = Console::default();
		type_text(&mut console, "st x");
		console.handle(key(KeyCode::Home));
		console.handle(key(KeyCode::Right));
		type_text(&mut console, "e");
		console.handle(key(KeyCode::End));
		console.handle(key(KeyCode::Backspace));
		type_text(&mut console, "y");
		assert_eq!(console.handle(key(KeyCode::Enter)), Some(Action::Run("set y".to_string())));
		assert_eq!(console.line, "");
	}

	#[test]
	fn up_and_down_recall_earlier_commands() {
		let configuration = Configuration::new();
		let inspector = inspector(configuration);
		let mut console = Console::default();
		for line in ["help", "set a.b 1"] {
			console.run_line(line, &inspector, &LoopWaker::default(), Instant::now());
		}

		console.handle(key(KeyCode::Up));
		assert_eq!(console.line, "set a.b 1");
		console.handle(key(KeyCode::Up));
		assert_eq!(console.line, "help");
		console.handle(key(KeyCode::Down));
		assert_eq!(console.line, "set a.b 1");
		console.handle(key(KeyCode::Down));
		assert_eq!(console.line, "");
	}

	#[test]
	fn ctrl_c_closes_the_application() {
		let mut console = Console::default();
		assert_eq!(
			console.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
			Some(Action::Quit)
		);
	}

	#[test]
	fn set_reports_the_value_the_owning_system_applied() {
		let configuration = Configuration::new();
		let port = configuration.register("render.lod.");
		let inspector = inspector(configuration);
		let mut console = Console::default();
		let start = Instant::now();

		console.run_line("set render.lod.force 2", &inspector, &LoopWaker::default(), start);
		assert_eq!(console.status, "");

		let update = port.read().expect("queued update");
		assert_eq!(update.parameter(), "force");
		port.set(update.id(), update.value().clone());
		console.collect_results(&inspector, start);
		assert_eq!(console.status, "render.lod.force set to 2");
	}

	#[test]
	fn set_reports_why_an_unowned_parameter_was_not_set() {
		let inspector = inspector(Configuration::new());
		let mut console = Console::default();
		console.run_line("set audio.gain 0.5", &inspector, &LoopWaker::default(), Instant::now());
		assert!(console.status.starts_with("audio.gain not set: "), "{}", console.status);
		assert!(console.status.contains("no system registered"), "{}", console.status);
	}

	#[test]
	fn an_unanswered_set_is_pending_after_a_second_and_reported_once_answered() {
		let configuration = Configuration::new();
		let port = configuration.register("render.pass.");
		let inspector = inspector(configuration);
		let mut console = Console::default();
		let start = Instant::now();

		console.run_line("set render.pass.bloom bypassed", &inspector, &LoopWaker::default(), start);
		console.collect_results(&inspector, start + RESULT_WAIT);
		assert!(
			console.status.starts_with("render.pass.bloom is pending."),
			"{}",
			console.status
		);

		let update = port.read().expect("queued update");
		port.set(update.id(), update.value().clone());
		console.collect_results(&inspector, start + RESULT_WAIT * 2);
		assert_eq!(console.status, "render.pass.bloom set to bypassed");
	}

	#[test]
	fn the_log_pane_wraps_long_lines_and_shows_the_newest_rows() {
		let lines = VecDeque::from([console_line("old"), console_line("abcdefghij"), console_line("new")]);
		let mut console = Console::default();
		// A 7-wide terminal leaves 5 columns and, under 2 border rows, 3 rows in the pane.
		let rows = draw(&mut console, &lines, 7, 7);
		assert_eq!(rows[1..4], ["│abcde│", "│fghij│", "│new  │"]);

		// A page up past the oldest row stops with the oldest row at the top.
		console.handle(key(KeyCode::PageUp));
		let rows = draw(&mut console, &lines, 7, 7);
		assert_eq!(rows[1..4], ["│old  │", "│abcde│", "│fghij│"]);

		console.handle(key(KeyCode::PageDown));
		let rows = draw(&mut console, &lines, 7, 7);
		assert_eq!(rows[3], "│new  │");
	}

	#[test]
	fn the_command_line_and_status_line_show_under_the_log() {
		let mut console = Console::default();
		type_text(&mut console, "help");
		let rows = draw(&mut console, &VecDeque::new(), 40, 6);
		assert!(rows[4].starts_with("> help"), "{rows:?}");
		assert!(rows[5].starts_with("Type `help` to list the commands"), "{rows:?}");
	}

	#[test]
	fn only_the_level_of_a_record_is_colored() {
		let record = Entry {
			kind: LineKind::Log(log::Level::Error),
			text: "12:00:00 ERROR app  it broke\nsecond line".to_string(),
		};
		let mut console = Console::default();
		let buffer = draw_buffer(&mut console, &VecDeque::from([record]), 40, 6);
		// Row 1 is the record's first row inside the border; column 1 is its first character.
		let color = |x: u16, y: u16| buffer[(x + 1, y)].fg;
		assert_eq!(color(0, 1), Color::Reset, "time");
		assert!((9..14).all(|x| color(x, 1) == Color::Red), "level");
		assert!((15..40 - 2).all(|x| color(x, 1) == Color::Reset), "target and message");
		assert!((0..40 - 2).all(|x| color(x, 2) == Color::Reset), "continuation line");
		assert_eq!(buffer[(1, 2)].symbol(), "s");
	}
}
