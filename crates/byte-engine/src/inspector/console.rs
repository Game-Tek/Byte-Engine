//! The terminal console: a full-screen view of the application's log or its bakes, with a command line under it.
//!
//! The console is a second transport over the [`DefaultInspector`], beside the
//! [`HttpInspectorServer`](super::http::HttpInspectorServer). [`GraphicsApplication`] starts it with [`spawn`]. Its
//! `set` command changes a runtime setting through [`DefaultInspector::update_configuration`]; `entities` finds
//! entities through [`DefaultInspector::entities`], and `post` changes one through [`DefaultInspector::post_message`].
//! See the [console reference](/docs/reference/console) for the screen, the keys, and the commands.
//!
//! [`GraphicsApplication`]: crate::application::graphics::GraphicsApplication

use std::{
	collections::VecDeque,
	io::{self, IsTerminal as _, Write as _},
	time::{Duration, Instant},
};

use neo_frizbee::{Config, Matcher, Pattern};
use ratatui::{
	DefaultTerminal, Frame,
	crossterm::{
		event::{
			self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
			PushKeyboardEnhancementFlags,
		},
		execute, terminal,
	},
	layout::{Constraint, Layout, Position, Rect},
	style::{Color, Style, Stylize as _},
	text::{Line, Span},
	widgets::{Block, Clear, Paragraph},
};
use resource_management::asset::bake_state::BakeState;

use crate::{
	application::{
		Events, LoopWaker,
		log_capture::{self, Entry, LineKind},
		parameters::Parameters,
		thread::Thread,
	},
	configuration::{Accepts, ConfigurationEventId, ConfigurationUpdateState, ConfigurationValue},
	core::{EntityHandle, factory::Handle, listener::DefaultListener, listener::Listener as _},
	inspector::{DefaultInspector, InspectedEntity, InspectedValue, RegisteredMessageType},
};

/// How long `set` waits for the owning system before it reports the update as pending.
const RESULT_WAIT: Duration = Duration::from_secs(1);

/// How long the console waits for a key before it looks for new log lines, results, and the application closing.
const POLL: Duration = Duration::from_millis(50);

/// The `CommandSpec` struct is one console command as `help` lists it and completion offers it.
struct CommandSpec {
	name: &'static str,
	usage: &'static str,
	summary: &'static str,
}

/// Every command [`parse_line`] runs, in the order `help` lists them.
const COMMANDS: [CommandSpec; 4] = [
	CommandSpec {
		name: "set",
		usage: "set <parameter> <value>",
		summary: "Changes a runtime setting, such as `set render.lod.force 2`.",
	},
	CommandSpec {
		name: "entities",
		usage: "entities [search]",
		summary: "Shows the entities whose name matches the search, such as `entities crate type:camera`.",
	},
	CommandSpec {
		name: "post",
		usage: "post <type> <target> [payload]",
		summary: "Posts a message to an entity, such as `post TransformationUpdate 12 {\"position\":[0,1,0]}`.",
	},
	CommandSpec {
		name: "help",
		usage: "help",
		summary: "Lists the commands.",
	},
];

const KEYS: &str = "Tab complete · ⌘1-3 or F2 view · ↑↓ history · PgUp/PgDn log · Ctrl+C quit";

/// The most candidate rows the completion list shows above the command line.
const COMPLETION_ROWS: usize = 8;

/// How often the entities view refreshes, since entities move and spawn without the console hearing of it.
const ENTITIES_REFRESH: Duration = Duration::from_millis(250);

/// The stream the asset manager reports its bakes on; see the
/// [bake stream reference](/docs/reference/resource-management#bake-stream).
const BAKES_STREAM: &str = "bakes";

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
		// A terminal reports Cmd with a key only once asked to; one that can't keeps F2 for switching views.
		let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false)
			&& execute!(
				io::stdout(),
				PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
			)
			.is_ok();
		log_capture::begin();
		let result = run(&mut terminal, &inspector, &waker, &mut events);
		if enhanced {
			let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
		}
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
	let start = Instant::now();
	loop {
		while let Some(event) = events.read() {
			if event == Events::Close {
				return Ok(());
			}
		}
		let now = Instant::now();
		console.collect_results(inspector, now);
		// Redraw only when a line arrived, a key or resize changed the screen, a result came in, a stream record
		// arrived for the bakes view, a second passed while it shows bakes running, or the entities view is due to
		// refresh.
		let elapsed = now.saturating_duration_since(start);
		let view = console.shown_view();
		let revision = (
			log_capture::revision(),
			console.revision,
			(view == View::Bakes).then(|| inspector.streams().revision()),
			console.shows_running_bakes.then_some(elapsed.as_secs()),
			(view == View::Entities).then(|| elapsed.as_millis() / ENTITIES_REFRESH.as_millis()),
		);
		if drawn != Some(revision) {
			// Each view reads only its own source, so the log view never parses a record or lists entities.
			let bakes = (view == View::Bakes).then(|| latest_bakes(inspector)).flatten();
			let entities = if view == View::Entities {
				inspector.entities(None, None)
			} else {
				Vec::new()
			};
			terminal.draw(|frame| {
				log_capture::with_entries(|entries| console.draw(frame, entries, bakes.as_ref(), &entities, now))
			})?;
			drawn = Some(revision);
		}
		if !event::poll(POLL)? {
			continue;
		}
		match event::read()? {
			Event::Key(key) if key.kind == KeyEventKind::Press => match console.handle(key) {
				Some(Action::Run(line)) => console.run_line(&line, inspector, waker, Instant::now()),
				Some(Action::Complete { forward }) => {
					let parameters = inspector.configuration.parameters();
					let entities = inspector.entities(None, None);
					let messages = inspector.message_types();
					let sources = Sources {
						parameters: &parameters,
						entities: &entities,
						messages: &messages,
					};
					console.complete(&sources, forward);
				}
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

/// Returns the asset manager's latest bake state the inspector kept, with when it arrived.
fn latest_bakes(inspector: &DefaultInspector) -> Option<(Instant, BakeState)> {
	let (received, mut record) = inspector.streams().latest(BAKES_STREAM)?;
	let state = serde_json::from_value(record["data"].take()).ok()?;
	Some((received, state))
}

/// The `View` enum is what the console's pane shows; Cmd and a view's number shows it, and F2 shows the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
	#[default]
	Log,
	Bakes,
	Entities,
}

impl View {
	/// Every view in the order of their numbers, from 1.
	const ALL: [Self; 3] = [Self::Log, Self::Bakes, Self::Entities];

	/// Returns the view a digit key names, such as `2` for the bakes.
	fn numbered(digit: char) -> Option<Self> {
		let index = digit.to_digit(10)?.checked_sub(1)?;
		Self::ALL.get(index as usize).copied()
	}

	/// Returns the view after this one in [`Self::ALL`], wrapping around.
	fn next(self) -> Self {
		Self::ALL[(self as usize + 1) % Self::ALL.len()]
	}

	fn title(self) -> &'static str {
		match self {
			Self::Log => "Log",
			Self::Bakes => "Bakes",
			Self::Entities => "Entities",
		}
	}
}

/// The `Sources` struct is what Tab completes from: the application's state at the moment Tab is pressed.
struct Sources<'a> {
	/// The declared runtime settings, sorted by name.
	parameters: &'a [(String, Accepts)],
	entities: &'a [InspectedEntity],
	/// The message types `post` accepts.
	messages: &'a [RegisteredMessageType<'a>],
}

/// The `Action` enum is what a key asks of the console loop.
#[derive(Debug, PartialEq, Eq)]
enum Action {
	/// Run this command line.
	Run(String),
	/// Complete the word before the cursor, or select the next candidate, or the previous one when not `forward`.
	Complete { forward: bool },
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
	view: View,
	/// Whether the last draw showed bakes running, whose times advance each second.
	shows_running_bakes: bool,
	/// The candidates the last Tab offered, while no other key has been pressed since.
	completion: Option<Completion>,
	/// The search of the last `entities` command, which filters the entities view.
	entity_search: String,
}

/// The `Completion` struct is the list Tab opened over the command line, which further presses of Tab cycle through.
#[derive(Debug, Default)]
struct Completion {
	/// The character index in the command line where the completed word starts. A candidate replaces the line from
	/// here to the cursor.
	start: usize,
	candidates: Vec<Candidate>,
	/// The candidate in the command line, once Tab has cycled to one.
	selected: Option<usize>,
	/// What the word accepts, such as "render.motion-blur.taps takes a whole number from 1 through 32".
	hint: Option<String>,
}

/// The `Candidate` struct is one word the completed word can become.
#[derive(Debug, PartialEq)]
struct Candidate {
	text: String,
	/// What the candidate is or accepts, shown dim beside it.
	note: String,
	/// Whether the word is whole, so a space follows it, rather than a name segment the next Tab continues.
	whole: bool,
}

impl Console {
	/// Applies one key press to the command line, the history, or the log pane, and returns what the loop must do.
	fn handle(&mut self, key: KeyEvent) -> Option<Action> {
		self.revision += 1;
		match key.code {
			KeyCode::Tab => return Some(Action::Complete { forward: true }),
			KeyCode::BackTab => return Some(Action::Complete { forward: false }),
			// Any other key changes the line or leaves it, so the list no longer matches it.
			_ => self.completion = None,
		}
		if key.modifiers.contains(KeyModifiers::SUPER) {
			// Cmd keys never type: Cmd and a view's number shows that view, and the rest do nothing.
			if let KeyCode::Char(digit) = key.code
				&& let Some(view) = View::numbered(digit)
			{
				self.view = view;
			}
			return None;
		}
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
			KeyCode::F(2) => self.view = self.shown_view().next(),
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

	/// Completes the word before the cursor from the commands and the declared `parameters`.
	///
	/// One candidate replaces the word. Several replace it with the text they all start with and open the list; while
	/// the list is open, each further call puts the next candidate, or the previous one when not `forward`, in the line.
	fn complete(&mut self, sources: &Sources, forward: bool) {
		self.revision += 1;
		if let Some(completion) = self.completion.as_mut().filter(|completion| completion.candidates.len() > 1) {
			let count = completion.candidates.len();
			let selected = match (completion.selected, forward) {
				(None, true) => 0,
				(None, false) => count - 1,
				(Some(index), true) => (index + 1) % count,
				(Some(index), false) => (index + count - 1) % count,
			};
			completion.selected = Some(selected);
			let start = completion.start;
			let replacement = completion.candidates[selected].insertion();
			self.replace_word(start, &replacement);
			return;
		}

		let before: String = self.line.chars().take(self.cursor).collect();
		let (start, candidates, hint) = candidates(&before, sources);
		match candidates.as_slice() {
			[only] => {
				let replacement = only.insertion();
				self.replace_word(start, &replacement);
				self.completion = None;
			}
			[] => {
				self.completion = hint.map(|hint| Completion {
					start,
					hint: Some(hint),
					..Completion::default()
				});
			}
			[first, rest @ ..] => {
				let shared = rest.iter().fold(first.text.as_str(), |shared, candidate| {
					let length = shared
						.char_indices()
						.zip(candidate.text.chars())
						.find(|((_, a), b)| a != b)
						.map_or(shared.len().min(candidate.text.len()), |((at, _), _)| at);
					&shared[..length]
				});
				// A fuzzy candidate, such as a handle found by name, needn't start with the word; the text the
				// candidates share then isn't the word made longer, so the word stays as typed.
				let word: String = before.chars().skip(start).collect();
				if starts_with_ignore_case(shared, &word) {
					self.replace_word(start, shared);
				}
				self.completion = Some(Completion {
					start,
					candidates,
					selected: None,
					hint,
				});
			}
		}
	}

	/// Replaces the command line from character `start` to the cursor with `text` and puts the cursor after it.
	fn replace_word(&mut self, start: usize, text: &str) {
		let (from, to) = (self.byte_index(start), self.byte_index(self.cursor));
		self.line.replace_range(from..to, text);
		self.cursor = start + text.chars().count();
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
			Ok(Command::Help) => self.report(help()),
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
			Ok(Command::Entities { search }) => {
				self.entity_search = search.to_string();
				self.view = View::Entities;
			}
			Ok(Command::Post {
				message_type,
				target,
				payload,
			}) => match inspector.post_message(message_type, target, payload) {
				Ok(()) => {
					// The world handles the message on a tick, which an idle application only runs when woken.
					waker.wake();
					self.report(format!("Posted {message_type} to {}", target.id()));
				}
				Err(error) => self.report(error),
			},
			Err(error) => self.report(error),
		}
	}

	/// Returns the rest of the command line when it's an `entities` command being typed.
	fn typed_entity_search(&self) -> Option<&str> {
		match split_word(self.line.trim_start()) {
			("entities", search) => Some(search),
			_ => None,
		}
	}

	/// Returns the view the pane shows: the entities while an `entities` command is being typed, so the lookup shows
	/// each key's result, and otherwise the view F2 chose.
	fn shown_view(&self) -> View {
		if self.typed_entity_search().is_some() {
			View::Entities
		} else {
			self.view
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

	/// Draws the pane's view, the command line, and the status line. `bakes` is the latest bake state with when it
	/// arrived, `entities` are the entities the entities view filters, and `now` is when the frame is drawn, which
	/// running bakes' times count to.
	fn draw(
		&mut self,
		frame: &mut Frame,
		entries: &VecDeque<Entry>,
		bakes: Option<&(Instant, BakeState)>,
		entities: &[InspectedEntity],
		now: Instant,
	) {
		let [pane_area, command_area, status_area] =
			Layout::vertical([Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)]).areas(frame.area());

		let view = self.shown_view();
		// The title lists every view with its number, like tabs, and marks the one shown.
		let tabs: Vec<Span> = View::ALL
			.iter()
			.enumerate()
			.map(|(index, &tab)| {
				let label = Span::raw(format!(" ⌘{} {} ", index + 1, tab.title()));
				if tab == view { label.reversed() } else { label.dim() }
			})
			.collect();
		let block = Block::bordered().title(Line::from(tabs));
		let inner = block.inner(pane_area);
		frame.render_widget(block, pane_area);
		self.shows_running_bakes = false;
		match view {
			View::Log => {
				self.page = inner.height as usize;
				let rows = self.log_rows(entries, inner);
				frame.render_widget(Paragraph::new(rows), inner);
			}
			View::Bakes => {
				self.shows_running_bakes = bakes.is_some_and(|(_, state)| !state.baking.is_empty());
				// Rows longer than the pane are cut, since a paragraph without wrapping clips them.
				frame.render_widget(Paragraph::new(bake_rows(bakes, now)), inner);
			}
			View::Entities => {
				// While an `entities` command is being typed, the view follows each key; otherwise it keeps the last
				// command's search.
				let search = self.typed_entity_search().unwrap_or(&self.entity_search);
				let found = find_entities(entities, search);
				// Rows past the pane's bottom are cut; a narrower search brings an entity into view.
				let rows = entity_rows(&found, entities.len(), search, inner.height as usize);
				frame.render_widget(Paragraph::new(rows), inner);
			}
		}

		if let Some(completion) = &self.completion {
			draw_completion(frame, completion, inner);
		}

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

/// Returns the bakes view's rows for `bakes`, the latest bake state and when it arrived, as of `now`: the totals, the
/// running bakes with how long each has run, and the latest bakes to end.
///
/// Both lists are trees: each bake sits under the ID that asked for it, named relative to that ID, so a row says why
/// its asset bakes. A requester that isn't in the list, such as a resource whose freshness is being checked, is a
/// dim row without a time.
fn bake_rows(bakes: Option<&(Instant, BakeState)>, now: Instant) -> Vec<Line<'_>> {
	let Some((received, state)) = bakes else {
		return vec![Line::raw("No bake was reported yet.").dim()];
	};
	// A record holds each bake's time when it was written; the bake kept running since.
	let since_received = now.saturating_duration_since(*received).as_secs_f64() * 1000.0;
	let mut rows = Vec::with_capacity(state.baking.len() + state.finished.len() + 4);
	rows.push(Line::raw(format!(
		"{} baking · {} baked · {} failed",
		state.baking.len(),
		state.baked,
		state.failed
	)));
	if !state.baking.is_empty() {
		rows.push(Line::raw("Baking").bold());
		push_tree(
			&mut rows,
			state
				.baking
				.iter()
				.map(|bake| (bake.requested_by.as_slice(), bake.id.as_str(), bake.elapsed_ms)),
			|elapsed_ms| vec![Span::raw(format!("  {:>7}", show_milliseconds(elapsed_ms + since_received)))],
		);
	}
	if !state.finished.is_empty() {
		rows.push(Line::raw("Finished").bold());
		push_tree(
			&mut rows,
			state
				.finished
				.iter()
				.map(|bake| (bake.requested_by.as_slice(), bake.id.as_str(), (bake.ok, bake.duration_ms))),
			|(ok, duration_ms)| {
				let mark = if ok {
					Span::styled("✓", Style::new().fg(Color::Green))
				} else {
					Span::styled("✗", Style::new().fg(Color::Red))
				};
				vec![mark, Span::raw(format!(" {:>7}", show_milliseconds(duration_ms)))]
			},
		);
	}
	rows
}

/// Pushes a row for each of `bakes`, its requesters outermost first, its ID, and its value, nested under its
/// requesters. `columns` draws a bake's value as the nine columns before its ID; a requester that isn't in `bakes`
/// leaves them blank and shows dim.
fn push_tree<'a, T: Copy>(
	rows: &mut Vec<Line<'a>>,
	bakes: impl Iterator<Item = (&'a [String], &'a str, T)>,
	columns: impl Fn(T) -> Vec<Span<'a>>,
) {
	BakeTree::new(bakes).visit(|guide, label, value| {
		// A requester that isn't in the list has no value: blank columns, and the whole row dim.
		let mut spans = value.map_or_else(|| vec![Span::raw(" ".repeat(9))], &columns);
		// The guide is dim, so the tree's lines lead the eye without competing with the names.
		spans.extend([Span::raw(" "), Span::raw(guide.to_string()).dim(), Span::raw(label)]);
		let line = Line::from(spans);
		rows.push(if value.is_some() { line } else { line.dim() });
	});
}

/// The `BakeTree` struct arranges a list of bakes under the IDs that asked for them, for the bakes view to draw.
///
/// Build it with [`BakeTree::new`], then walk it in drawing order with [`BakeTree::visit`].
struct BakeTree<'a, T> {
	nodes: Vec<BakeNode<'a, T>>,
	/// The nodes without a requester, in the order of the list.
	roots: Vec<usize>,
}

/// One ID in a [`BakeTree`]: a bake from the list, or a requester the list doesn't hold.
struct BakeNode<'a, T> {
	id: &'a str,
	/// The bake's value, or `None` for a requester that isn't in the list.
	value: Option<T>,
	children: Vec<usize>,
}

impl<'a, T: Copy> BakeTree<'a, T> {
	/// Arranges `bakes`, each its requesters outermost first, its ID, and its value, keeping the list's order among
	/// siblings.
	fn new(bakes: impl Iterator<Item = (&'a [String], &'a str, T)>) -> Self {
		let mut tree = Self {
			nodes: Vec::new(),
			roots: Vec::new(),
		};
		for (requested_by, id, value) in bakes {
			// Walk down the requesters, adding the ones no earlier bake brought in.
			let mut parent = None;
			for requester in requested_by {
				parent = Some(tree.child(parent, requester, |node| node.id == requester));
			}
			// An ID can be in the list twice, such as a bake that ended and ran again, so each bake gets its own row.
			let leaf = tree.child(parent, id, |node| node.id == id && node.value.is_none());
			tree.nodes[leaf].value = Some(value);
		}
		tree
	}

	/// Returns the child of `parent`, or the root when `parent` is `None`, that `matches`, adding one for `id` when none
	/// does.
	fn child(&mut self, parent: Option<usize>, id: &'a str, matches: impl Fn(&BakeNode<'a, T>) -> bool) -> usize {
		let siblings = match parent {
			Some(parent) => &self.nodes[parent].children,
			None => &self.roots,
		};
		if let Some(&found) = siblings.iter().find(|&&index| matches(&self.nodes[index])) {
			return found;
		}
		let index = self.nodes.len();
		self.nodes.push(BakeNode {
			id,
			value: None,
			children: Vec::new(),
		});
		match parent {
			Some(parent) => self.nodes[parent].children.push(index),
			None => self.roots.push(index),
		}
		index
	}

	/// Calls `row` for each node, parents before their children: with its guide, the tree's lines from the root down to
	/// it, its ID relative to its parent's, and its value.
	fn visit(&self, mut row: impl FnMut(&str, &'a str, Option<T>)) {
		// One guide is grown and cut back as the walk goes down and up, so no row copies its parent's.
		let mut guide = String::new();
		for &root in &self.roots {
			self.visit_node(root, None, None, &mut guide, &mut row);
		}
	}

	/// Visits node `index` and the nodes under it. `last` is whether the node is the last of its siblings, or `None`
	/// for a root, which hangs from no line.
	fn visit_node(
		&self,
		index: usize,
		parent: Option<&'a str>,
		last: Option<bool>,
		guide: &mut String,
		row: &mut impl FnMut(&str, &'a str, Option<T>),
	) {
		let node = &self.nodes[index];
		let inherited = guide.len();
		// `├─` leads to a node with siblings below it; `╰─` to the last, rounded where the line ends.
		if let Some(last) = last {
			guide.push_str(if last { "╰─ " } else { "├─ " });
		}
		row(
			guide,
			parent.map_or(node.id, |parent| relative_id(parent, node.id)),
			node.value,
		);
		guide.truncate(inherited);
		// Under a node with siblings below it, the line runs on past its children; under the last, it already ended.
		if let Some(last) = last {
			guide.push_str(if last { "   " } else { "│  " });
		}
		for (position, &child) in node.children.iter().enumerate() {
			let last = position + 1 == node.children.len();
			self.visit_node(child, Some(node.id), Some(last), guide, row);
		}
		guide.truncate(inherited);
	}
}

/// Returns `id` without the part it shares with `parent`, so a row under its requester names only what differs: a
/// subresource keeps its `#fragment`, and a file keeps the path after the directories both share.
fn relative_id<'a>(parent: &str, id: &'a str) -> &'a str {
	let shared = parent
		.char_indices()
		.zip(id.chars())
		.find(|((_, a), b)| a != b)
		.map_or(parent.len().min(id.len()), |((at, _), _)| at);
	if shared == parent.len() && id[shared..].starts_with('#') {
		return &id[shared..];
	}
	// Cut after the last directory both IDs share, never inside a name.
	id[..shared].rfind('/').map_or(id, |slash| &id[slash + 1..])
}

/// Shows a time in milliseconds below a second and in seconds from there.
fn show_milliseconds(milliseconds: f64) -> String {
	if milliseconds < 1000.0 {
		format!("{milliseconds:.0}ms")
	} else {
		format!("{:.1}s", milliseconds / 1000.0)
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

/// Returns the `help` text: each command's usage and what it does.
fn help() -> String {
	let width = COMMANDS.iter().map(|command| command.usage.len()).max().unwrap_or(0);
	COMMANDS
		.iter()
		.map(|command| format!("{:width$}  {}", command.usage, command.summary))
		.collect::<Vec<_>>()
		.join("\n")
}

impl Candidate {
	/// Returns the text that replaces the completed word: a whole word with a space after it.
	fn insertion(&self) -> String {
		if self.whole {
			format!("{} ", self.text)
		} else {
			self.text.clone()
		}
	}
}

/// Returns what the last word of `before`, the command line up to the cursor, can become: the character index where
/// the word starts, the candidates in order, and what the word accepts when it's a value.
///
/// The first word completes to a command. The name after `set` completes to a declared parameter one dotted segment at
/// a time, so `render.` offers `render.gi.` and `render.lod.` before their settings. The value after a name completes
/// to the words its setting accepts. The word after `post` completes to a message type, and the next one looks up an
/// entity by name and completes to its handle.
fn candidates(before: &str, sources: &Sources) -> (usize, Vec<Candidate>, Option<String>) {
	let parameters = sources.parameters;
	let word = before.rsplit(char::is_whitespace).next().unwrap_or_default();
	let start = before.chars().count() - word.chars().count();
	let previous: Vec<&str> = before[..before.len() - word.len()].split_whitespace().collect();

	match previous.as_slice() {
		[] => {
			let candidates = COMMANDS
				.iter()
				.filter(|command| command.name.starts_with(word))
				.map(|command| Candidate {
					text: command.name.to_string(),
					note: command.summary.to_string(),
					whole: true,
				})
				.collect();
			(start, candidates, None)
		}
		["set"] => {
			let mut candidates: Vec<Candidate> = Vec::new();
			// A name with a space in it can't be typed as one word, so it isn't offered.
			for (name, accepts) in parameters
				.iter()
				.filter(|(name, _)| name.starts_with(word) && !name.contains(' '))
			{
				let candidate = match name[word.len()..].find('.') {
					Some(dot) => Candidate {
						text: name[..word.len() + dot + 1].to_string(),
						note: String::new(),
						whole: false,
					},
					None => Candidate {
						text: name.clone(),
						note: accepts.describe(),
						whole: true,
					},
				};
				// Names arrive sorted, so the names under one segment are next to each other.
				if candidates.last().is_none_or(|last| *last != candidate) {
					candidates.push(candidate);
				}
			}
			(start, candidates, None)
		}
		["set", name] => {
			let Some((_, accepts)) = parameters.iter().find(|(declared, _)| declared == name) else {
				return (start, Vec::new(), None);
			};
			let lowered = word.to_ascii_lowercase();
			let candidates = accepts
				.words()
				.iter()
				.filter(|accepted| accepted.to_ascii_lowercase().starts_with(&lowered))
				.map(|accepted| Candidate {
					text: accepted.to_string(),
					note: String::new(),
					whole: true,
				})
				.collect();
			(start, candidates, Some(format!("{name} takes {}", accepts.describe())))
		}
		["post"] => {
			let candidates = sources
				.messages
				.iter()
				.filter(|message| starts_with_ignore_case(message.message_type, word))
				.map(|message| Candidate {
					text: message.message_type.to_string(),
					note: String::new(),
					whole: true,
				})
				.collect();
			(start, candidates, None)
		}
		["post", _] => {
			let candidates = find_entities(sources.entities, word)
				.into_iter()
				.map(|entity| Candidate {
					text: entity.handle().id().to_string(),
					note: format!("{}  {}", entity_name(entity), short_types(entity)),
					whole: true,
				})
				.collect();
			(start, candidates, None)
		}
		["post", message_type, _] => {
			let hint = sources
				.messages
				.iter()
				.find(|message| message.message_type == *message_type)
				.map(|message| format!("{message_type} takes {}", message.payload_shape));
			(start, Vec::new(), hint)
		}
		_ => (start, Vec::new(), None),
	}
}

/// Draws the completion list over the bottom rows of `pane`, the pane's inside, just above the command line: the
/// candidates, the one in the line highlighted, then what the word accepts.
fn draw_completion(frame: &mut Frame, completion: &Completion, pane: Rect) {
	// Scroll the list so the selected candidate stays among the shown rows.
	let first = completion
		.selected
		.map_or(0, |selected| (selected + 1).saturating_sub(COMPLETION_ROWS));
	let mut rows: Vec<Line> = completion
		.candidates
		.iter()
		.enumerate()
		.skip(first)
		.take(COMPLETION_ROWS)
		.map(|(index, candidate)| {
			let mut spans = vec![Span::raw(candidate.text.as_str())];
			if !candidate.note.is_empty() {
				spans.push(Span::raw(format!("  {}", candidate.note)).dim());
			}
			let line = Line::from(spans);
			if completion.selected == Some(index) {
				line.reversed()
			} else {
				line
			}
		})
		.collect();
	let hidden = completion.candidates.len() - rows.len();
	if hidden > 0 {
		rows.push(Line::from(format!("… {hidden} more")).dim());
	}
	if let Some(hint) = &completion.hint {
		rows.push(Line::from(hint.as_str()).dim());
	}
	let height = (rows.len() as u16).min(pane.height);
	let area = Rect::new(pane.x, pane.bottom() - height, pane.width, height);
	frame.render_widget(Clear, area);
	frame.render_widget(Paragraph::new(rows), area);
}

/// Returns the entities that pass every word of `search`, an [entity search](/docs/reference/console#entity-search):
/// in handle order without a name word, and best match first with one.
///
/// A `type:` word keeps the entities with a type whose name contains its text; any other word fuzzy-matches names.
fn find_entities<'a>(entities: &'a [InspectedEntity], search: &str) -> Vec<&'a InspectedEntity> {
	let mut patterns = Vec::new();
	let mut types = Vec::new();
	for word in search.split_whitespace() {
		match word.strip_prefix("type:") {
			Some(text) => types.push(text),
			None => patterns.push(Pattern::parse(word)),
		}
	}
	let mut kept: Vec<&InspectedEntity> = entities
		.iter()
		.filter(|entity| {
			types
				.iter()
				.all(|text| entity.types().iter().any(|name| contains_ignore_case(name, text)))
		})
		.collect();
	kept.sort_unstable_by_key(|entity| entity.handle().id());
	if patterns.is_empty() {
		return kept;
	}
	let names: Vec<&str> = kept.iter().map(|entity| entity_name(entity)).collect();
	// The matcher sorts best match first, then by position in `names`, so equal scores stay in handle order.
	Matcher::from_patterns(&patterns, &Config::default())
		.match_list(&names)
		.into_iter()
		// An entity without a name has nothing to match, so only the negated forms could keep it; none does.
		.filter(|found| !names[found.index as usize].is_empty())
		.map(|found| kept[found.index as usize])
		.collect()
}

/// Returns the entities view's rows: how many entities `found` holds of `total`, then up to `height - 1` entities
/// with their handle, name, position, and types.
fn entity_rows<'a>(found: &[&'a InspectedEntity], total: usize, search: &str, height: usize) -> Vec<Line<'a>> {
	let mut rows = Vec::with_capacity(height.min(found.len() + 1));
	let summary = if search.trim().is_empty() {
		format!("{total} entities")
	} else {
		format!("{} of {total} entities match `{}`", found.len(), search.trim())
	};
	rows.push(Line::raw(summary).bold());
	let shown = &found[..found.len().min(height.saturating_sub(1))];
	// The name column is as wide as the longest shown name, up to a limit so the types stay on screen.
	let name_width = shown
		.iter()
		.map(|entity| entity_name(entity).chars().count())
		.max()
		.unwrap_or(0)
		.clamp(4, 32);
	for entity in shown {
		let position = match entity.property("position") {
			Some(InspectedValue::Point(point)) => format!("{:.2}, {:.2}, {:.2}", point.x(), point.y(), point.z()),
			_ => "-".to_string(),
		};
		rows.push(Line::from(vec![
			Span::raw(format!("{:>6}  ", entity.handle().id())),
			Span::raw(format!("{:<name_width$}  ", entity_name(entity))),
			Span::raw(format!("{position:<26}")),
			Span::raw(short_types(entity)).dim(),
		]));
	}
	rows
}

/// Returns the entity's name, or empty text when it has none.
fn entity_name(entity: &InspectedEntity) -> &str {
	match entity.property("name") {
		Some(InspectedValue::Text(name)) => name,
		_ => "",
	}
}

/// Returns the entity's types without their module paths, joined by commas, such as `Camera, Name`.
fn short_types(entity: &InspectedEntity) -> String {
	let mut shown = String::new();
	for (index, name) in entity.types().iter().enumerate() {
		if index > 0 {
			shown.push_str(", ");
		}
		push_short_type(&mut shown, name);
	}
	shown
}

/// Pushes the Rust type `name` with every path cut to its last segment, so `alloc::vec::Vec<core::option::Option<u8>>`
/// becomes `Vec<Option<u8>>`.
fn push_short_type(shown: &mut String, name: &str) {
	let mut path_start = 0;
	for (at, character) in name.char_indices() {
		// Paths end at the punctuation that separates them inside generic, tuple, array, and reference types.
		if matches!(character, '<' | '>' | ',' | ' ' | '(' | ')' | '[' | ']' | '&' | ';') {
			shown.push_str(last_segment(&name[path_start..at]));
			shown.push(character);
			path_start = at + character.len_utf8();
		}
	}
	shown.push_str(last_segment(&name[path_start..]));
}

/// Returns the last `::` segment of a path.
fn last_segment(path: &str) -> &str {
	path.rsplit("::").next().unwrap_or(path)
}

/// Returns whether `text` contains `part`, comparing ASCII letters in any case.
fn contains_ignore_case(text: &str, part: &str) -> bool {
	part.is_empty()
		|| text
			.as_bytes()
			.windows(part.len())
			.any(|window| window.eq_ignore_ascii_case(part.as_bytes()))
}

/// Returns whether `text` starts with `prefix`, comparing ASCII letters in any case.
fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
	text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// The `Command` enum is one parsed command line, borrowing its names from the line.
#[derive(Debug, PartialEq)]
enum Command<'a> {
	/// Changes the runtime setting named `parameter`.
	Set { parameter: &'a str, value: ConfigurationValue },
	/// Shows the entities view, filtered by `search`.
	Entities { search: &'a str },
	/// Posts a registered message to `target`, with `payload` as JSON.
	Post {
		message_type: &'a str,
		target: Handle,
		payload: &'a str,
	},
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
		"entities" => Ok(Command::Entities { search: arguments }),
		"post" => {
			let (message_type, rest) = split_word(arguments);
			let (target, payload) = split_word(rest);
			if message_type.is_empty() || target.is_empty() {
				return Err(
					"`post` needs a message type and a target. The most likely cause is that one of them is missing. Type `post <type> <target> [payload]`, such as `post Delete 12`."
						.to_string(),
				);
			}
			let target = target.parse::<u32>().ok().filter(|id| *id != 0).ok_or_else(|| {
				format!(
					"`post` target `{target}` isn't an entity handle. The most likely cause is a name typed where the handle goes. Press Tab after the name to replace it with the entity's handle."
				)
			})?;
			Ok(Command::Post {
				message_type,
				target: Handle::from_id(target),
				payload: if payload.is_empty() { "null" } else { payload },
			})
		}
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

	/// Draws `console` over `entries` and `bakes` and returns its buffer.
	fn draw_with(
		console: &mut Console,
		entries: &VecDeque<Entry>,
		bakes: Option<&(Instant, BakeState)>,
		width: u16,
		height: u16,
	) -> ratatui::buffer::Buffer {
		let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
		let now = bakes.map_or_else(Instant::now, |(received, _)| *received);
		terminal
			.draw(|frame| console.draw(frame, entries, bakes, &[], now))
			.expect("draw");
		terminal.backend().buffer().clone()
	}

	/// Draws `console` over `entries` and returns the text of each row.
	fn draw(console: &mut Console, entries: &VecDeque<Entry>, width: u16, height: u16) -> Vec<String> {
		rows(&draw_with(console, entries, None, width, height))
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
		let buffer = draw_with(&mut console, &VecDeque::from([record]), None, 40, 6);
		// Row 1 is the record's first row inside the border; column 1 is its first character.
		let color = |x: u16, y: u16| buffer[(x + 1, y)].fg;
		assert_eq!(color(0, 1), Color::Reset, "time");
		assert!((9..14).all(|x| color(x, 1) == Color::Red), "level");
		assert!((15..40 - 2).all(|x| color(x, 1) == Color::Reset), "target and message");
		assert!((0..40 - 2).all(|x| color(x, 2) == Color::Reset), "continuation line");
		assert_eq!(buffer[(1, 2)].symbol(), "s");
	}

	/// Returns the text of each row of `buffer`.
	fn rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
		(0..buffer.area.height)
			.map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
			.collect()
	}

	#[test]
	fn f2_switches_between_the_log_the_bakes_and_the_entities() {
		let mut console = Console::default();
		assert_eq!(console.view, View::Log);

		console.handle(key(KeyCode::F(2)));
		let shown = draw(&mut console, &VecDeque::new(), 40, 6);
		assert!(shown[1].contains("No bake was reported yet."), "{shown:?}");

		console.handle(key(KeyCode::F(2)));
		let shown = draw(&mut console, &VecDeque::new(), 40, 6);
		assert!(shown[1].contains("0 entities"), "{shown:?}");

		console.handle(key(KeyCode::F(2)));
		assert_eq!(console.view, View::Log);
	}

	#[test]
	fn cmd_and_a_number_shows_that_view_and_types_nothing() {
		let cmd = |digit| KeyEvent::new(KeyCode::Char(digit), KeyModifiers::SUPER);
		let mut console = Console::default();
		console.handle(cmd('3'));
		let buffer = draw_with(&mut console, &VecDeque::new(), None, 60, 6);
		let shown = rows(&buffer);
		assert!(shown[0].contains("⌘1 Log") && shown[0].contains("⌘3 Entities"), "{shown:?}");
		assert!(shown[1].contains("0 entities"), "{shown:?}");
		// The shown view's tab is marked.
		let tab = shown[0]
			.find("⌘3")
			.map(|at| shown[0][..at].chars().count() as u16)
			.expect("a tab");
		assert!(buffer[(tab, 0)].modifier.contains(ratatui::style::Modifier::REVERSED));

		console.handle(cmd('2'));
		assert!(draw(&mut console, &VecDeque::new(), 60, 6)[1].contains("No bake was reported yet."));
		console.handle(cmd('9'));
		assert_eq!(console.view, View::Bakes);
		assert_eq!(console.line, "");
	}

	/// The parameters a renderer might declare, sorted as [`Configuration::parameters`] returns them.
	fn parameters() -> Vec<(String, Accepts)> {
		[
			("render.gi.enabled", Accepts::Bool),
			("render.gi.technique", Accepts::Choice(&["ssgi", "surfels"])),
			("render.lod.bias", Accepts::Number { min: 0.0 }),
			("render.lod.force", Accepts::WholeOr("off")),
			("render.pass.atmosphere sky", Accepts::Choice(&["enabled", "bypassed"])),
		]
		.into_iter()
		.map(|(name, accepts)| (name.to_string(), accepts))
		.collect()
	}

	/// Completes `console`'s command line from [`parameters`] alone.
	fn complete(console: &mut Console, forward: bool) {
		let parameters = parameters();
		let sources = Sources {
			parameters: &parameters,
			entities: &[],
			messages: &[],
		};
		console.complete(&sources, forward);
	}

	/// Types `text`, presses Tab, and returns the command line.
	fn tab(console: &mut Console, text: &str) -> String {
		type_text(console, text);
		assert_eq!(console.handle(key(KeyCode::Tab)), Some(Action::Complete { forward: true }));
		complete(console, true);
		console.line.clone()
	}

	#[test]
	fn tab_completes_a_command() {
		let mut console = Console::default();
		assert_eq!(tab(&mut console, "se"), "set ");
	}

	#[test]
	fn tab_completes_a_parameter_name_one_segment_at_a_time() {
		let mut console = Console::default();
		assert_eq!(tab(&mut console, "set re"), "set render.");
		assert_eq!(tab(&mut console, "l"), "set render.lod.");
		assert_eq!(tab(&mut console, "f"), "set render.lod.force ");
		assert_eq!(tab(&mut console, "o"), "set render.lod.force off ");
	}

	#[test]
	fn tab_lists_the_candidates_and_cycles_through_them() {
		let mut console = Console::default();
		assert_eq!(tab(&mut console, "set render."), "set render.");
		let rows = draw(&mut console, &VecDeque::new(), 60, 8);
		// The list sits on the pane's bottom rows, above the command line. A name with a space can't be typed, so the
		// `atmosphere sky` pass isn't offered.
		assert_eq!(
			rows[3..5].iter().map(|row| row.trim_matches(['│', ' '])).collect::<Vec<_>>(),
			["render.gi.", "render.lod.",]
		);

		console.handle(key(KeyCode::Tab));
		complete(&mut console, true);
		assert_eq!(console.line, "set render.gi.");
		complete(&mut console, true);
		assert_eq!(console.line, "set render.lod.");
		complete(&mut console, false);
		assert_eq!(console.line, "set render.gi.");

		// Any other key closes the list.
		type_text(&mut console, "t");
		assert!(console.completion.is_none());
	}

	#[test]
	fn tab_completes_a_value_to_the_words_its_setting_accepts() {
		let mut console = Console::default();
		assert_eq!(
			tab(&mut console, "set render.gi.technique SU"),
			"set render.gi.technique surfels "
		);

		let mut console = Console::default();
		// Both words start with `s`, so Tab types it and lists them.
		assert_eq!(tab(&mut console, "set render.gi.technique "), "set render.gi.technique s");
		let rows = draw(&mut console, &VecDeque::new(), 80, 8);
		assert!(rows[2].contains("ssgi") && rows[3].contains("surfels"), "{rows:?}");
		assert!(rows[4].contains("render.gi.technique takes `ssgi` or `surfels`"), "{rows:?}");
	}

	#[test]
	fn tab_describes_a_number_it_cannot_list() {
		let mut console = Console::default();
		assert_eq!(tab(&mut console, "set render.lod.bias "), "set render.lod.bias ");
		let rows = draw(&mut console, &VecDeque::new(), 80, 6);
		assert!(rows[2].contains("render.lod.bias takes a number of 0 or more"), "{rows:?}");
	}

	#[test]
	fn the_bakes_view_shows_the_totals_running_bakes_and_finished_bakes() {
		use resource_management::asset::bake_state::{BakingAsset, FinishedBake};

		let state = BakeState {
			baking: vec![BakingAsset {
				id: "models/tree.glb".to_string(),
				requested_by: Vec::new(),
				elapsed_ms: 2400.0,
			}],
			baked: 41,
			failed: 1,
			finished: vec![
				FinishedBake {
					id: "materials/bark.bema".to_string(),
					requested_by: Vec::new(),
					ok: true,
					duration_ms: 120.0,
				},
				FinishedBake {
					id: "shaders/broken.besl".to_string(),
					requested_by: Vec::new(),
					ok: false,
					duration_ms: 15.0,
				},
			],
		};
		let mut console = Console::default();
		console.handle(key(KeyCode::F(2)));
		let buffer = draw_with(&mut console, &VecDeque::new(), Some(&(Instant::now(), state)), 48, 10);
		let shown = rows(&buffer);

		assert!(shown[1].contains("1 baking · 41 baked · 1 failed"), "{shown:?}");
		assert!(shown[2].contains("Baking"), "{shown:?}");
		assert!(shown[3].contains("2.4s models/tree.glb"), "{shown:?}");
		assert!(shown[4].contains("Finished"), "{shown:?}");
		assert!(shown[5].contains("✓   120ms materials/bark.bema"), "{shown:?}");
		assert!(shown[6].contains("✗    15ms shaders/broken.besl"), "{shown:?}");
		assert_eq!(buffer[(1, 5)].fg, Color::Green);
		assert_eq!(buffer[(1, 6)].fg, Color::Red);
	}

	#[test]
	fn the_bakes_view_nests_each_bake_under_its_requesters_relative_to_them() {
		use resource_management::asset::bake_state::{BakingAsset, FinishedBake};

		let ids = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
		let state = BakeState {
			baking: vec![
				BakingAsset {
					id: "models/tree.glb".to_string(),
					requested_by: ids(&["levels/forest.bel"]),
					elapsed_ms: 2400.0,
				},
				BakingAsset {
					id: "models/tree.glb#bark".to_string(),
					requested_by: ids(&["levels/forest.bel", "models/tree.glb"]),
					elapsed_ms: 300.0,
				},
				BakingAsset {
					id: "models/textures/bark.png".to_string(),
					requested_by: ids(&["levels/forest.bel", "models/tree.glb", "models/tree.glb#bark"]),
					elapsed_ms: 100.0,
				},
				BakingAsset {
					id: "models/tree.glb#leaves".to_string(),
					requested_by: ids(&["levels/forest.bel", "models/tree.glb"]),
					elapsed_ms: 80.0,
				},
			],
			finished: vec![FinishedBake {
				id: "models/leaf.png".to_string(),
				requested_by: ids(&["models/tree.glb"]),
				ok: true,
				duration_ms: 50.0,
			}],
			..BakeState::default()
		};
		let mut console = Console::default();
		console.handle(key(KeyCode::F(2)));
		let buffer = draw_with(&mut console, &VecDeque::new(), Some(&(Instant::now(), state)), 60, 16);
		let shown = rows(&buffer);

		// The level only waits on the model, so it's a dim row without a time.
		assert!(shown[3].contains("│          levels/forest.bel "), "{shown:?}");
		assert!(buffer[(11, 3)].modifier.contains(ratatui::style::Modifier::DIM));
		// The model shares no directory with the level, so it keeps its whole ID; it's the level's only bake, so its
		// branch ends there.
		assert!(shown[4].contains("2.4s ╰─ models/tree.glb"), "{shown:?}");
		// The bark has a sibling below it, so its line runs on past the bark's own bakes to the leaves.
		assert!(shown[5].contains("300ms    ├─ #bark"), "{shown:?}");
		assert!(shown[6].contains("100ms    │  ╰─ textures/bark.png"), "{shown:?}");
		assert!(shown[7].contains("80ms    ╰─ #leaves"), "{shown:?}");
		// The guides are dim; the names aren't.
		let guide = shown[5]
			.find('├')
			.map(|at| shown[5][..at].chars().count() as u16)
			.expect("a branch");
		assert!(buffer[(guide, 5)].modifier.contains(ratatui::style::Modifier::DIM));
		assert!(!buffer[(guide + 3, 5)].modifier.contains(ratatui::style::Modifier::DIM));
		assert!(shown[9].contains("models/tree.glb"), "{shown:?}");
		assert!(shown[10].contains("✓    50ms ╰─ leaf.png"), "{shown:?}");
	}

	/// The `World` struct is an inspector over a world scope that tests spawn entities into and whose
	/// `TransformationUpdate` messages `post` can send.
	struct World {
		inspector: DefaultInspector,
		scope: crate::core::message_bus::MessageScope,
	}

	impl World {
		fn new() -> Self {
			let message_bus = MessageBus::default();
			message_bus.observe().expect("attach test message observer");
			let scope = message_bus.new_scope("console-entities-test-world");
			let mut inspector = DefaultInspector::new(
				crate::core::channel::DefaultChannel::new(),
				Configuration::new(),
				scope.clone(),
				Arc::new(Metrics::new()),
			);
			inspector
				.register_message(
					crate::inspector::TRANSFORMATION_UPDATE_MESSAGE_TYPE,
					scope.channel::<crate::gameplay::TransformationUpdate>(),
				)
				.expect("register transformation updates");
			Self { inspector, scope }
		}

		/// Spawns an entity with `name` at `position` and returns its handle.
		fn spawn(&self, name: &str, position: [f32; 3]) -> Handle {
			use crate::core::Creator as _;
			let [x, y, z] = position;
			self.scope
				.create(String::from("model"))
				.with(crate::gameplay::Name::new(name.to_string()))
				.with(crate::gameplay::Transform::from_position(math::Point::new(x, y, z)))
				.into()
		}

		/// Types `text` into `console`, presses Tab, completes from this world, and returns the command line.
		fn tab(&self, console: &mut Console, text: &str) -> String {
			type_text(console, text);
			console.handle(key(KeyCode::Tab));
			let entities = self.inspector.entities(None, None);
			let messages = self.inspector.message_types();
			let sources = Sources {
				parameters: &[],
				entities: &entities,
				messages: &messages,
			};
			console.complete(&sources, true);
			console.line.clone()
		}

		/// Draws `console` with this world's entities and returns the text of each row.
		fn draw(&self, console: &mut Console, width: u16, height: u16) -> Vec<String> {
			let entities = self.inspector.entities(None, None);
			let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
			terminal
				.draw(|frame| console.draw(frame, &VecDeque::new(), None, &entities, Instant::now()))
				.expect("draw");
			rows(terminal.backend().buffer())
		}
	}

	#[test]
	fn the_entities_view_filters_by_name_as_the_search_is_typed() {
		let world = World::new();
		let crate_handle = world.spawn("shipping crate", [1.0, 2.0, 3.0]);
		world.spawn("main camera", [0.0, 0.0, 0.0]);
		world.spawn("barrel", [0.0, 0.0, 0.0]);
		let mut console = Console::default();

		type_text(&mut console, "entities shpcrt");
		let shown = world.draw(&mut console, 80, 8);
		assert!(shown[0].contains("Entities"), "{shown:?}");
		assert!(shown[1].contains("1 of 3 entities match `shpcrt`"), "{shown:?}");
		let id = crate_handle.id().to_string();
		assert!(
			shown[2].contains(&id) && shown[2].contains("shipping crate") && shown[2].contains("1.00, 2.00, 3.00"),
			"{shown:?}"
		);
		assert!(shown[2].contains("String"), "types lose their module paths: {shown:?}");
		assert!(!shown.iter().any(|row| row.contains("barrel")), "{shown:?}");

		// Enter keeps the search once the command line is empty again.
		assert_eq!(
			console.handle(key(KeyCode::Enter)),
			Some(Action::Run("entities shpcrt".to_string()))
		);
		console.run_line("entities shpcrt", &world.inspector, &LoopWaker::default(), Instant::now());
		assert!(world.draw(&mut console, 80, 8)[1].contains("1 of 3 entities match"));

		console.run_line("entities", &world.inspector, &LoopWaker::default(), Instant::now());
		let shown = world.draw(&mut console, 80, 8);
		assert!(shown[1].contains("3 entities"), "{shown:?}");
		assert!(
			shown[2].contains("shipping crate"),
			"handle order puts the first spawn first: {shown:?}"
		);
	}

	#[test]
	fn a_type_word_keeps_entities_with_a_matching_type() {
		let world = World::new();
		world.spawn("crate", [0.0; 3]);
		let mut console = Console::default();
		type_text(&mut console, "entities type:string");
		assert!(world.draw(&mut console, 80, 6)[1].contains("1 of 1 entities match"));

		let mut console = Console::default();
		type_text(&mut console, "entities type:camera");
		assert!(world.draw(&mut console, 80, 6)[1].contains("0 of 1 entities match"));
	}

	#[test]
	fn post_completes_a_target_by_name_and_moves_the_entity() {
		let world = World::new();
		let handle = world.spawn("shipping crate", [1.0, 2.0, 3.0]);
		world.spawn("barrel", [0.0; 3]);
		let mut console = Console::default();

		assert_eq!(world.tab(&mut console, "post tra"), "post TransformationUpdate ");
		let line = world.tab(&mut console, "crate");
		assert_eq!(line, format!("post TransformationUpdate {} ", handle.id()));

		// With the target typed, Tab shows the payload's shape.
		world.tab(&mut console, "");
		let shown = world.draw(&mut console, 120, 8);
		assert!(
			shown.iter().any(|row| row.contains("TransformationUpdate takes")),
			"{shown:?}"
		);

		type_text(&mut console, r#"{"position":[4,5,6]}"#);
		let Some(Action::Run(line)) = console.handle(key(KeyCode::Enter)) else {
			panic!("Enter runs the line");
		};
		console.run_line(&line, &world.inspector, &LoopWaker::default(), Instant::now());
		assert_eq!(console.status, format!("Posted TransformationUpdate to {}", handle.id()));
		let moved = world.inspector.entities(None, Some("shipping crate"));
		assert_eq!(
			moved[0].property("position"),
			Some(InspectedValue::Point(math::Point::new(4.0, 5.0, 6.0)))
		);
	}

	#[test]
	fn tab_after_a_name_matching_several_entities_keeps_the_name_and_lists_them() {
		let world = World::new();
		world.spawn("crate a", [0.0; 3]);
		world.spawn("crate b", [0.0; 3]);
		let mut console = Console::default();
		assert_eq!(
			world.tab(&mut console, "post TransformationUpdate crate"),
			"post TransformationUpdate crate"
		);
		let shown = world.draw(&mut console, 80, 8);
		assert!(shown.iter().any(|row| row.contains("crate a")), "{shown:?}");
		assert!(shown.iter().any(|row| row.contains("crate b")), "{shown:?}");
	}

	#[test]
	fn post_reports_a_payload_that_does_not_match_its_shape() {
		let world = World::new();
		let handle = world.spawn("crate", [0.0; 3]);
		let mut console = Console::default();
		let line = format!("post TransformationUpdate {} {{\"speed\":1}}", handle.id());
		console.run_line(&line, &world.inspector, &LoopWaker::default(), Instant::now());
		assert!(
			console.status.starts_with("Inspector message payload is invalid."),
			"{}",
			console.status
		);
	}

	#[test]
	fn post_needs_a_type_and_a_numeric_target() {
		assert!(parse_line("post Delete").is_err());
		assert!(
			parse_line("post Delete crate")
				.unwrap_err()
				.contains("isn't an entity handle")
		);
		assert!(parse_line("post Delete 0").is_err());
		assert!(matches!(
			parse_line("post Delete 7"),
			Ok(Command::Post { payload: "null", .. })
		));
	}

	#[test]
	fn short_types_drop_module_paths_inside_generics() {
		let mut shown = String::new();
		push_short_type(&mut shown, "alloc::vec::Vec<core::option::Option<(u8, my::Thing)>>");
		assert_eq!(shown, "Vec<Option<(u8, Thing)>>");
	}
}
