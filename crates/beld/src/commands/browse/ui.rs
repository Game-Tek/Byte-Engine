//! Draw the browser in the terminal and turn key presses into changes to what it shows.

use std::{borrow::Cow, collections::HashSet, fmt::Write as _};

use ratatui::{
	Frame,
	crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
	layout::{Constraint, Layout, Rect},
	style::{Color, Modifier, Style, Stylize as _},
	text::{Line, Span},
	widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Row as TableRow, Table},
};
#[cfg(debug_assertions)]
use resource_management::{ResourceTraceItem, ResourceTraceLevel};
use serde_json::Value as Json;

use crate::commands::{
	catalog::{Catalog, Field, SHOWN_FIELDS, Status, ratio, share},
	filter::Filter,
	shared::plural,
	view::{Group, NO_VALUE_GROUP, Sort, view},
};

/// The `Exit` enum tells the command loop what to do after the browser returns.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Exit {
	Quit,
	/// Read the store again and keep browsing with the same filter, group, and sort.
	Reload,
}

/// The `TableLine` enum is one line of the table: a group header or a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableLine {
	Group(usize),
	Row(usize),
}

/// The `SelectionKey` enum names the selected table line by its group name or resource ID, which stay meaningful
/// after the indexes in [`TableLine`] change.
enum SelectionKey {
	Group(String),
	Row(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
	Table,
	Detail,
}

/// The `PickerEntry` enum is one choice in the group or sort picker.
#[derive(Clone, Debug, PartialEq)]
enum PickerEntry {
	/// A field, or no field to stop grouping.
	Field(Option<Field>),
	/// Prompts for a `model.` path.
	Model,
}

/// The `Picker` struct is an open group or sort picker.
struct Picker {
	for_group: bool,
	entries: Vec<PickerEntry>,
	selected: usize,
	/// The model path being typed, after the `model.` entry was chosen.
	model_path: Option<String>,
}

/// The `DetailLine` struct is one line of the detail pane, and the row Enter jumps to from it.
struct DetailLine {
	line: Line<'static>,
	target: Option<usize>,
}

impl DetailLine {
	fn plain(line: impl Into<Line<'static>>) -> DetailLine {
		DetailLine {
			line: line.into(),
			target: None,
		}
	}
}

/// The `State` struct holds everything the browser shows besides the catalog, so it survives a reload.
pub(super) struct State {
	filter_text: String,
	filter: Filter,
	/// The filter text from before the current edit, while the filter is being edited.
	editing: Option<String>,
	group: Option<Field>,
	sort: Sort,
	/// The names of the expanded groups.
	expanded: HashSet<String>,
	groups: Vec<Group>,
	lines: Vec<TableLine>,
	selected: usize,
	offset: usize,
	focus: Focus,
	/// The ID the detail pane shows after a jump through a reference; `None` follows the table selection.
	subject: Option<String>,
	/// The IDs shown before each jump, for `Backspace`.
	history: Vec<String>,
	detail: Vec<DetailLine>,
	/// The table line and pane width the detail lines were built for.
	detail_for: Option<(TableLine, u16)>,
	/// The detail pane's inner width at the last draw, which trace lines wrap to.
	detail_width: u16,
	detail_selected: usize,
	detail_offset: usize,
	picker: Option<Picker>,
	/// The usage pane's lines while it's open. They change only when the store is read again.
	usage: Option<Vec<Line<'static>>>,
	/// A notice or error for the status line, cleared by the next key.
	message: Option<String>,
}

impl State {
	/// Starts with no filter, no group, and the largest stored resources first.
	pub(super) fn new(catalog: &mut Catalog) -> State {
		let mut state = State {
			filter_text: String::new(),
			filter: Filter::default(),
			editing: None,
			group: None,
			sort: Sort {
				field: Field::Stored,
				reversed: false,
			},
			expanded: HashSet::new(),
			groups: Vec::new(),
			lines: Vec::new(),
			selected: 0,
			offset: 0,
			focus: Focus::Table,
			subject: None,
			history: Vec::new(),
			detail: Vec::new(),
			detail_for: None,
			detail_width: 80,
			detail_selected: 0,
			detail_offset: 0,
			picker: None,
			usage: None,
			message: None,
		};
		state.refresh(catalog);
		state
	}

	/// Replaces the catalog with a newly read one and applies the current filter, group, and sort to it.
	pub(super) fn reload(&mut self, catalog: &mut Catalog, reloaded: Catalog) {
		// The selection is named before the rows it indexes go away.
		let previous = self.selection_key(catalog);
		*catalog = reloaded;
		// Row indexes name different resources in the new catalog.
		self.detail_for = None;
		self.message = None;
		match Filter::parse(&self.filter_text, catalog) {
			Ok(filter) => self.filter = filter,
			Err(error) => {
				self.filter = Filter::default();
				self.message = Some(error);
			}
		}
		if self.usage.is_some() {
			self.usage = Some(usage_lines(catalog));
		}
		self.refresh_from(catalog, previous);
	}

	/// Shows a message on the status line until the next key.
	pub(super) fn notify(&mut self, message: String) {
		self.message = Some(message);
	}

	/// Handles one key press and returns whether the browser should stop.
	///
	/// The detail lines follow the new selection before this returns, so the next key moves through the lines of
	/// the resource on screen.
	pub(super) fn handle(&mut self, catalog: &mut Catalog, key: KeyEvent) -> Option<Exit> {
		let exit = self.apply(catalog, key);
		self.sync_detail(catalog);
		exit
	}

	fn apply(&mut self, catalog: &mut Catalog, key: KeyEvent) -> Option<Exit> {
		if key.kind != KeyEventKind::Press {
			return None;
		}
		self.message = None;
		if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
			return Some(Exit::Quit);
		}
		if self.editing.is_some() {
			self.edit_filter(catalog, key.code);
			return None;
		}
		if self.picker.is_some() {
			self.pick(catalog, key.code);
			return None;
		}
		if let Some(by) = key_step(key.code) {
			self.move_selection(by);
			return None;
		}

		match key.code {
			KeyCode::Char('q') => return Some(Exit::Quit),
			KeyCode::Char('R') => return Some(Exit::Reload),
			KeyCode::Char('/') => self.editing = Some(self.filter_text.clone()),
			KeyCode::Char('g') => self.open_picker(catalog, true),
			KeyCode::Char('s') => self.open_picker(catalog, false),
			KeyCode::Char('r') => {
				self.sort.reversed = !self.sort.reversed;
				self.refresh(catalog);
			}
			KeyCode::Char('u') => {
				self.usage = match self.usage {
					Some(_) => None,
					None => Some(usage_lines(catalog)),
				}
			}
			KeyCode::Tab => {
				self.focus = match self.focus {
					Focus::Table => Focus::Detail,
					Focus::Detail => Focus::Table,
				}
			}
			KeyCode::Esc => self.focus = Focus::Table,
			KeyCode::Backspace => self.back(catalog),
			KeyCode::Enter => self.enter(catalog),
			_ => {}
		}
		None
	}

	/// Edits the filter text, applying it after every key while it parses.
	fn edit_filter(&mut self, catalog: &mut Catalog, code: KeyCode) {
		match code {
			KeyCode::Enter => {
				match Filter::parse(&self.filter_text, catalog) {
					Ok(_) => self.editing = None,
					Err(error) => self.message = Some(error),
				}
				return;
			}
			KeyCode::Esc => {
				self.filter_text = self.editing.take().unwrap_or_default();
			}
			KeyCode::Backspace => {
				self.filter_text.pop();
			}
			KeyCode::Char(character) => self.filter_text.push(character),
			_ => return,
		}

		match Filter::parse(&self.filter_text, catalog) {
			Ok(filter) => {
				self.filter = filter;
				self.refresh(catalog);
			}
			// The view keeps the last valid filter while the text is incomplete.
			Err(error) => self.message = Some(error),
		}
	}

	fn open_picker(&mut self, catalog: &Catalog, for_group: bool) {
		let current = PickerEntry::Field(if for_group {
			self.group.clone()
		} else {
			Some(self.sort.field.clone())
		});
		let entries = for_group
			.then_some(PickerEntry::Field(None))
			.into_iter()
			.chain(
				Field::all(&catalog.properties)
					.into_iter()
					.map(|field| PickerEntry::Field(Some(field))),
			)
			.chain(std::iter::once(PickerEntry::Model))
			.collect::<Vec<_>>();
		let selected = entries
			.iter()
			.position(|entry| *entry == current)
			.unwrap_or(entries.len() - 1);
		self.picker = Some(Picker {
			for_group,
			entries,
			selected,
			model_path: None,
		});
	}

	fn pick(&mut self, catalog: &mut Catalog, code: KeyCode) {
		let Some(picker) = self.picker.as_mut() else {
			return;
		};

		if let Some(path) = picker.model_path.as_mut() {
			match code {
				KeyCode::Char(character) => path.push(character),
				KeyCode::Backspace => {
					path.pop();
				}
				KeyCode::Esc => picker.model_path = None,
				KeyCode::Enter => match Field::parse(&format!("model.{path}"), &catalog.properties) {
					Ok(field) => {
						let for_group = picker.for_group;
						self.picker = None;
						self.choose(catalog, for_group, Some(field));
					}
					Err(error) => self.message = Some(error),
				},
				_ => {}
			}
			return;
		}

		if let Some(by) = key_step(code) {
			picker.selected = step(picker.selected, by, picker.entries.len());
			return;
		}
		match code {
			KeyCode::Esc => self.picker = None,
			KeyCode::Enter => match &picker.entries[picker.selected] {
				PickerEntry::Model => picker.model_path = Some(String::new()),
				PickerEntry::Field(field) => {
					let (for_group, field) = (picker.for_group, field.clone());
					self.picker = None;
					self.choose(catalog, for_group, field);
				}
			},
			_ => {}
		}
	}

	/// Applies a picked group or sort field. A newly picked sort runs in its field's usual direction.
	fn choose(&mut self, catalog: &mut Catalog, for_group: bool, field: Option<Field>) {
		if for_group {
			self.group = field;
			self.expanded.clear();
		} else if let Some(field) = field {
			self.sort = Sort { field, reversed: false };
		}
		self.refresh(catalog);
	}

	fn enter(&mut self, catalog: &mut Catalog) {
		match self.focus {
			Focus::Table => match self.lines.get(self.selected) {
				Some(TableLine::Group(group)) => {
					let name = self.groups[*group].name.clone().unwrap_or_default();
					if !self.expanded.remove(&name) {
						self.expanded.insert(name);
					}
					let previous = self.selection_key(catalog);
					self.rebuild_lines(catalog, previous);
				}
				Some(TableLine::Row(_)) => self.focus = Focus::Detail,
				None => {}
			},
			Focus::Detail => {
				let Some(target) = self.detail.get(self.detail_selected).and_then(|line| line.target) else {
					return;
				};
				if let Some(current) = self.detail_row(catalog) {
					self.history.push(catalog.rows[current].id.clone());
				}
				let id = catalog.rows[target].id.clone();
				self.show(catalog, id);
			}
		}
	}

	/// Returns to the resource shown before the last jump.
	fn back(&mut self, catalog: &mut Catalog) {
		if let Some(previous) = self.history.pop() {
			self.show(catalog, previous);
		}
	}

	/// Shows a resource in the detail pane and selects it in the table when the filter shows it.
	fn show(&mut self, catalog: &mut Catalog, id: String) {
		if let Some(row) = catalog.row_of(&id) {
			// Expand the group that holds the resource so the table can select it.
			if let Some(group) = self.groups.iter().find(|group| group.rows.contains(&row))
				&& let Some(name) = &group.name
			{
				self.expanded.insert(name.clone());
				let previous = self.selection_key(catalog);
				self.rebuild_lines(catalog, previous);
			}
			match self.lines.iter().position(|line| *line == TableLine::Row(row)) {
				Some(line) => self.selected = line,
				None => self.message = Some(format!("The filter hides '{id}'. The detail pane shows it anyway.")),
			}
		}
		self.subject = Some(id);
	}

	fn move_selection(&mut self, by: isize) {
		match self.focus {
			Focus::Table => {
				self.selected = step(self.selected, by, self.lines.len());
				self.subject = None;
			}
			Focus::Detail => self.detail_selected = step(self.detail_selected, by, self.detail.len()),
		}
	}

	/// Recomputes the groups and table lines, keeping the selected line when it still exists.
	///
	/// The detail pane goes back to following the table, so it never describes a resource the new view left behind.
	fn refresh(&mut self, catalog: &mut Catalog) {
		let previous = self.selection_key(catalog);
		self.refresh_from(catalog, previous);
	}

	/// Recomputes the groups and table lines, selecting the line `previous` names when it still exists.
	fn refresh_from(&mut self, catalog: &mut Catalog, previous: Option<SelectionKey>) {
		self.subject = None;
		// Group indexes name different groups after the view changes; a resource's detail stays the same.
		if let Some((TableLine::Group(_), _)) = self.detail_for {
			self.detail_for = None;
		}
		self.groups = view(catalog, &self.filter, self.group.as_ref(), &self.sort);
		self.rebuild_lines(catalog, previous);
	}

	/// Names the selected line, so the selection survives changes to the groups and rows that lines index.
	fn selection_key(&self, catalog: &Catalog) -> Option<SelectionKey> {
		Some(match *self.lines.get(self.selected)? {
			TableLine::Group(group) => SelectionKey::Group(self.groups[group].name.clone().unwrap_or_default()),
			TableLine::Row(row) => SelectionKey::Row(catalog.rows[row].id.clone()),
		})
	}

	fn rebuild_lines(&mut self, catalog: &Catalog, previous: Option<SelectionKey>) {
		self.lines.clear();
		for (index, group) in self.groups.iter().enumerate() {
			match &group.name {
				None => self.lines.extend(group.rows.iter().map(|&row| TableLine::Row(row))),
				Some(name) => {
					self.lines.push(TableLine::Group(index));
					if self.expanded.contains(name) {
						self.lines.extend(group.rows.iter().map(|&row| TableLine::Row(row)));
					}
				}
			}
		}
		self.selected = previous
			.and_then(|key| {
				self.lines.iter().position(|line| match (line, &key) {
					(TableLine::Group(group), SelectionKey::Group(name)) => {
						self.groups[*group].name.as_deref().unwrap_or_default() == name
					}
					(TableLine::Row(row), SelectionKey::Row(id)) => catalog.rows[*row].id == *id,
					_ => false,
				})
			})
			.unwrap_or(0)
			.min(self.lines.len().saturating_sub(1));
	}

	/// Returns the row the detail pane shows: the last jump's target, or the selected resource.
	fn detail_row(&self, catalog: &Catalog) -> Option<usize> {
		match self.detail_line(catalog)? {
			TableLine::Row(row) => Some(row),
			TableLine::Group(_) => None,
		}
	}

	/// Returns the line the detail pane describes: the last jump's target, or the selected table line.
	fn detail_line(&self, catalog: &Catalog) -> Option<TableLine> {
		match &self.subject {
			Some(id) => catalog.row_of(id).map(TableLine::Row),
			None => self.lines.get(self.selected).copied(),
		}
	}

	/// Draws the whole browser.
	pub(super) fn draw(&mut self, frame: &mut Frame, catalog: &Catalog) {
		let [header, filter, settings, main, status] = Layout::vertical([
			Constraint::Length(1),
			Constraint::Length(1),
			Constraint::Length(1),
			Constraint::Fill(1),
			Constraint::Length(1),
		])
		.areas(frame.area());

		frame.render_widget(Paragraph::new(header_line(catalog)), header);
		frame.render_widget(Paragraph::new(self.filter_line()), filter);
		frame.render_widget(Paragraph::new(self.settings_line()), settings);
		frame.render_widget(Paragraph::new(self.status_line()), status);

		if let Some(usage) = &self.usage {
			frame.render_widget(Paragraph::new(usage.clone()).block(Block::bordered().title(" usage ")), main);
		} else {
			let [table, detail] = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(main);
			self.draw_table(frame, catalog, table);
			self.draw_detail(frame, catalog, detail);
		}

		if self.picker.is_some() {
			self.draw_picker(frame, main);
		}
	}

	fn filter_line(&self) -> Line<'_> {
		let mut spans = vec![
			Span::styled("filter> ", Style::new().fg(Color::Cyan)),
			Span::raw(self.filter_text.as_str()),
		];
		if self.editing.is_some() {
			spans.push(Span::styled("█", Style::new().add_modifier(Modifier::SLOW_BLINK)));
		}
		Line::from(spans)
	}

	fn settings_line(&self) -> Line<'_> {
		let direction = if self.sort.descending() { "↓" } else { "↑" };
		Line::from(vec![
			Span::styled("group: ", Style::new().fg(Color::Cyan)),
			Span::raw(self.group.as_ref().map_or(Cow::Borrowed("none"), Field::name)),
			Span::styled("   sort: ", Style::new().fg(Color::Cyan)),
			Span::raw(format!("{} {direction}", self.sort.field.name())),
		])
	}

	fn status_line(&self) -> Line<'_> {
		match &self.message {
			Some(message) => Line::styled(message.as_str(), Style::new().fg(Color::Yellow)),
			None if self.editing.is_some() => Line::raw(" ⏎ keep  esc restore").dim(),
			None => {
				Line::raw(" / filter  g group  s sort  r reverse  ⏎ open  ⇥ focus  ⌫ back  u usage  R reload  q quit").dim()
			}
		}
	}

	fn draw_table(&mut self, frame: &mut Frame, catalog: &Catalog, area: Rect) {
		// Two border lines and one header line; only the visible window of lines is built.
		let height = usize::from(area.height.saturating_sub(3)).max(1);
		self.offset = scroll(self.offset, self.selected, height);

		// The sort field gets a column when the table doesn't already show it.
		let extra = (!SHOWN_FIELDS.contains(&self.sort.field)).then_some(&self.sort.field);
		let extra_width = if extra.is_some() { EXTRA_WIDTH } else { 0 };
		// The ID column takes what the borders, the column gaps, and the fixed columns leave.
		let id_width = usize::from(area.width).saturating_sub(2 + 5 + usize::from(FIXED_WIDTH + extra_width));
		let focused = self.focus == Focus::Table;
		let rows = self
			.lines
			.iter()
			.enumerate()
			.skip(self.offset)
			.take(height)
			.map(|(index, line)| {
				let row = match *line {
					TableLine::Group(group) => {
						let group = &self.groups[group];
						let name = group.name.as_deref().unwrap_or(NO_VALUE_GROUP);
						let marker = if self.expanded.contains(name) { "▾" } else { "▸" };
						TableRow::new([
							format!("{marker} {}", fit(name, id_width.saturating_sub(2))),
							plural(group.rows.len(), "resource"),
							size(group.stored),
							size(group.size),
							format!("{:.0}%", share(group.stored, catalog.stored_total()) * 100.0),
						])
						.bold()
					}
					TableLine::Row(row) => {
						let entry = &catalog.rows[row];
						let cells = SHOWN_FIELDS.iter().chain(extra).map(|field| match field {
							Field::Id => Cow::Owned(format!("  {}", fit(&entry.id, id_width.saturating_sub(2)))),
							field => catalog.cell(row, field),
						});
						let style = match entry.status {
							Status::Failed => Style::new().fg(Color::Red),
							Status::Stale => Style::new().fg(Color::Yellow),
							Status::Current | Status::Unknown => Style::new(),
						};
						TableRow::new(cells).style(style)
					}
				};
				if index == self.selected {
					row.style(if focused {
						Style::new().reversed()
					} else {
						Style::new().add_modifier(Modifier::UNDERLINED)
					})
				} else {
					row
				}
			})
			.collect::<Vec<_>>();

		let shown = self.groups.iter().map(|group| group.rows.len()).sum::<usize>();
		let title = format!(" {shown} of {} ", catalog.rows.len());
		let header = TableRow::new(SHOWN_FIELDS.iter().chain(extra).map(Field::name)).style(Style::new().fg(Color::Cyan));
		let widths = [
			Constraint::Fill(1),
			Constraint::Length(CLASS_WIDTH),
			Constraint::Length(STORED_WIDTH),
			Constraint::Length(ENCODING_WIDTH),
			Constraint::Length(RATIO_WIDTH),
			Constraint::Length(extra_width),
		];
		frame.render_widget(
			Table::new(rows, widths).header(header).block(Block::bordered().title(title)),
			area,
		);
	}

	/// Rebuilds the detail lines when the row they describe or the pane width changed.
	fn sync_detail(&mut self, catalog: &Catalog) {
		let line = self.detail_line(catalog);
		let built_for = line.map(|line| (line, self.detail_width));
		if built_for == self.detail_for {
			return;
		}
		if line != self.detail_for.map(|(line, _)| line) {
			self.detail_selected = 0;
			self.detail_offset = 0;
		}
		self.detail = match line {
			Some(TableLine::Row(row)) => detail_lines(catalog, row, usize::from(self.detail_width)),
			Some(TableLine::Group(group)) => group_lines(catalog, &self.groups[group]),
			None => Vec::new(),
		};
		self.detail_for = built_for;
	}

	fn draw_detail(&mut self, frame: &mut Frame, catalog: &Catalog, area: Rect) {
		self.detail_width = area.width.saturating_sub(2);
		self.sync_detail(catalog);

		// Only the visible window of lines is copied into the list.
		let height = usize::from(area.height.saturating_sub(2)).max(1);
		let focused = self.focus == Focus::Detail;
		if focused {
			self.detail_offset = scroll(self.detail_offset, self.detail_selected, height);
		}
		let items = self
			.detail
			.iter()
			.enumerate()
			.skip(self.detail_offset)
			.take(height)
			.map(|(index, line)| {
				let item = ListItem::new(line.line.clone());
				let item = if line.target.is_some() { item.fg(Color::Blue) } else { item };
				if focused && index == self.detail_selected {
					item.reversed()
				} else {
					item
				}
			});
		let block = Block::bordered().title(" detail ");
		frame.render_widget(
			List::new(items).block(if focused { block.fg(Color::Cyan) } else { block }),
			area,
		);
	}

	fn draw_picker(&self, frame: &mut Frame, area: Rect) {
		let Some(picker) = &self.picker else {
			return;
		};
		let width = 40.min(area.width);
		let height = (picker.entries.len() as u16 + 2).min(area.height);
		let popup = Rect::new(
			area.x + (area.width - width) / 2,
			area.y + (area.height - height) / 2,
			width,
			height,
		);
		let title = if picker.for_group { " group by " } else { " sort by " };

		frame.render_widget(Clear, popup);
		if let Some(path) = &picker.model_path {
			frame.render_widget(
				Paragraph::new(format!("model.{path}█")).block(Block::bordered().title(" model path ")),
				Rect::new(popup.x, popup.y, popup.width, 3.min(popup.height)),
			);
			return;
		}
		let items = picker.entries.iter().map(|entry| {
			ListItem::new(match entry {
				PickerEntry::Field(None) => Cow::Borrowed("none"),
				PickerEntry::Field(Some(field)) => field.name(),
				PickerEntry::Model => Cow::Borrowed("model.…"),
			})
		});
		let mut list_state = ListState::default().with_selected(Some(picker.selected));
		frame.render_stateful_widget(
			List::new(items)
				.block(Block::bordered().title(title))
				.highlight_style(Style::new().reversed()),
			popup,
			&mut list_state,
		);
	}
}

// The widths of the table's fixed columns. `ParticleSystem` and `metal-io-lz4` are the longest class and encoding.
const CLASS_WIDTH: u16 = 14;
const STORED_WIDTH: u16 = 10;
const ENCODING_WIDTH: u16 = 12;
const RATIO_WIDTH: u16 = 6;
const EXTRA_WIDTH: u16 = 14;
const FIXED_WIDTH: u16 = CLASS_WIDTH + STORED_WIDTH + ENCODING_WIDTH + RATIO_WIDTH;

/// Shortens text to `width` characters by replacing its middle with `…`, so both the source and the fragment of an ID
/// stay readable.
fn fit(text: &str, width: usize) -> Cow<'_, str> {
	let length = text.chars().count();
	if length <= width {
		return Cow::Borrowed(text);
	}
	let kept = width.saturating_sub(1);
	let head = kept / 3;
	let tail = text.chars().skip(length - (kept - head));
	Cow::Owned(
		text.chars()
			.take(head)
			.chain(std::iter::once('…'))
			.chain(tail)
			.take(width)
			.collect(),
	)
}

/// Breaks text into lines of at most `width` characters at spaces, splitting words longer than a line.
#[cfg(debug_assertions)]
fn wrap(text: &str, width: usize) -> Vec<String> {
	let width = width.max(1);
	let mut lines = Vec::new();
	let (mut line, mut line_length) = (String::new(), 0);
	for mut word in text.split(' ') {
		loop {
			let word_length = word.chars().count();
			let needed = if line_length == 0 {
				word_length
			} else {
				line_length + 1 + word_length
			};
			if needed <= width {
				if line_length > 0 {
					line.push(' ');
				}
				line.push_str(word);
				line_length = needed;
				break;
			}
			if line_length > 0 {
				lines.push(std::mem::take(&mut line));
				line_length = 0;
				continue;
			}
			// The word alone is longer than a line.
			let split = word.char_indices().nth(width).map_or(word.len(), |(index, _)| index);
			lines.push(word[..split].to_string());
			word = &word[split..];
		}
	}
	if line_length > 0 || lines.is_empty() {
		lines.push(line);
	}
	lines
}

/// Returns how far a navigation key moves a selection, or `None` for any other key.
fn key_step(code: KeyCode) -> Option<isize> {
	match code {
		KeyCode::Up => Some(-1),
		KeyCode::Down => Some(1),
		KeyCode::PageUp => Some(-20),
		KeyCode::PageDown => Some(20),
		KeyCode::Home => Some(isize::MIN),
		KeyCode::End => Some(isize::MAX),
		_ => None,
	}
}

/// Moves an index by `by`, clamped to `0..len`.
fn step(index: usize, by: isize, len: usize) -> usize {
	index.saturating_add_signed(by).min(len.saturating_sub(1))
}

/// Returns the first line of a window `height` lines tall, moved as little as possible to keep `selected` in it.
fn scroll(offset: usize, selected: usize, height: usize) -> usize {
	if selected < offset {
		selected
	} else if selected >= offset + height {
		selected + 1 - height
	} else {
		offset
	}
}

fn header_line(catalog: &Catalog) -> Line<'static> {
	let resources = catalog.rows.iter().filter_map(|row| row.resource.as_ref());
	let (count, size_sum) = resources.fold((0, 0), |(count, size_sum), resource| {
		(count + 1, size_sum + resource.size() as u64)
	});
	let mode = if catalog.usage.pack.is_some() { "packed" } else { "files" };
	Line::from(format!(
		" resources ({mode}) · {} · {} stored · {} decoded · {}",
		plural(count, "resource"),
		size(catalog.stored_total()),
		size(size_sum),
		bake_summary(catalog)
	))
	.bold()
}

/// Describes how many resources are stale and, in debug builds, how many bakes failed.
fn bake_summary(catalog: &Catalog) -> String {
	let count = |status: Status| catalog.rows.iter().filter(|row| row.status == status).count();
	let mut summary = if catalog.sources_checked {
		format!("{} stale", count(Status::Stale))
	} else {
		"sources not checked".to_string()
	};
	if cfg!(debug_assertions) {
		let _ = write!(summary, " · {} failed", count(Status::Failed));
	}
	summary
}

/// Lists the totals of one group for the detail pane.
fn group_lines(catalog: &Catalog, group: &Group) -> Vec<DetailLine> {
	let share = share(group.stored, catalog.stored_total());
	[
		Line::from(group.name.clone().unwrap_or_default()).bold(),
		Line::from(plural(group.rows.len(), "resource")),
		Line::from(format!("size {} · stored {}", size(group.size), size(group.stored))),
		Line::from(format!("ratio {:.2}", ratio(group.stored, group.size))),
		Line::from(format!("share of the store {:.0}% {}", share * 100.0, bar(share, 20))),
	]
	.into_iter()
	.map(DetailLine::plain)
	.collect()
}

/// Adds a blank line and a section title to the detail pane.
fn section(lines: &mut Vec<DetailLine>, title: impl Into<Cow<'static, str>>) {
	lines.push(DetailLine::plain(Line::default()));
	lines.push(DetailLine::plain(Line::from(title.into()).fg(Color::Cyan)));
}

/// Lists everything the detail pane shows for one row.
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn detail_lines(catalog: &Catalog, row: usize, width: usize) -> Vec<DetailLine> {
	let entry = &catalog.rows[row];
	let mut lines = vec![DetailLine::plain(Line::from(entry.id.clone()).bold())];

	if let Some(resource) = &entry.resource {
		let cell = |field: Field| catalog.cell(row, &field);
		lines.extend(
			[
				format!("{} · {} · {}", cell(Field::Class), cell(Field::Encoding), cell(Field::Status)),
				format!("size {} · stored {}", cell(Field::Size), cell(Field::Stored)),
				format!("ratio {} · metadata {}", cell(Field::Ratio), cell(Field::Metadata)),
				format!("baked {} ago · hash {}", cell(Field::Baked), cell(Field::Hash)),
			]
			.map(DetailLine::plain),
		);
		#[cfg(debug_assertions)]
		lines.push(DetailLine::plain(format!("total {}", cell(Field::Total))));

		if let Some(streams) = resource.streams().filter(|streams| !streams.is_empty()) {
			section(&mut lines, "streams");
			let mut streams = streams.iter().collect::<Vec<_>>();
			streams.sort_by_key(|stream| std::cmp::Reverse(stream.size()));
			for stream in streams {
				lines.push(DetailLine::plain(format!(
					"  {:<28} {:>10} {}",
					stream.name(),
					size(stream.size() as u64),
					bar(share(stream.size() as u64, resource.size() as u64), 10)
				)));
			}
		}

		if !entry.sources.is_empty() {
			section(&mut lines, "sources");
			for source in &entry.sources {
				let mark = if !catalog.sources_checked {
					"?"
				} else if entry.stale_sources.contains(source) {
					"changed"
				} else {
					"current"
				};
				lines.push(DetailLine::plain(format!("  {source} · {mark}")));
			}
		}

		if entry.root {
			lines.push(DetailLine::plain(Line::from("root: matches resources.roots")));
		}
		// Release builds read no references.
		if cfg!(debug_assertions) {
			if catalog.has_roots && !entry.reachable {
				lines.push(DetailLine::plain(
					Line::from("unreachable: no root uses this resource").fg(Color::Yellow),
				));
			}
			section(&mut lines, format!("uses ({})", entry.references.len()));
			for id in &entry.references {
				lines.push(match catalog.row_of(id) {
					Some(target) => DetailLine {
						line: reference_line(catalog, target),
						target: Some(target),
					},
					None => DetailLine::plain(Line::from(format!("  {id} · missing")).fg(Color::Red)),
				});
			}
			section(&mut lines, format!("used by ({})", entry.used_by.len()));
			for &user in &entry.used_by {
				lines.push(DetailLine {
					line: reference_line(catalog, user),
					target: Some(user),
				});
			}
		}
	}

	#[cfg(debug_assertions)]
	trace_lines(&entry.trace, width, &mut lines);

	if let Some(resource) = &entry.resource
		&& let Ok(Json::Object(model)) = resource_management::inspect::inspect_model(resource)
	{
		section(&mut lines, "model");
		let mut model_lines = Vec::new();
		for (key, value) in &model {
			json_lines(value, Label::Key(key), 1, &mut model_lines);
		}
		lines.extend(model_lines.into_iter().map(DetailLine::plain));
	}

	lines
}

/// Lists a bake trace, wrapped to the pane width.
#[cfg(debug_assertions)]
fn trace_lines(trace: &[ResourceTraceItem], width: usize, lines: &mut Vec<DetailLine>) {
	if trace.is_empty() {
		return;
	}
	section(lines, "trace");
	for item in trace {
		let color = match item.level() {
			ResourceTraceLevel::Info => Color::Reset,
			ResourceTraceLevel::Warn => Color::Yellow,
			ResourceTraceLevel::Error => Color::Red,
		};
		// Trace messages explain failures in full sentences, so they wrap instead of being cut off.
		for line in wrap(
			&format!("{}: {}", item.level().as_str(), item.message()),
			width.saturating_sub(2),
		) {
			lines.push(DetailLine::plain(Line::raw(format!("  {line}")).fg(color)));
		}
	}
}

/// Describes a referenced or referencing resource in one line.
fn reference_line(catalog: &Catalog, row: usize) -> Line<'static> {
	let entry = &catalog.rows[row];
	Line::from(match &entry.resource {
		Some(resource) => format!(
			"  {} · {} · {}",
			entry.id,
			resource.class(),
			size(resource.stored_size() as u64)
		),
		None => format!("  {} · failed", entry.id),
	})
}

/// The most model lines the detail pane shows, so a large model can't flood it.
const MAX_MODEL_LINES: usize = 400;
/// The longest array of numbers printed in full; longer ones, such as embedded snapshot bytes, show their length.
const MAX_INLINE_NUMBERS: usize = 8;

/// The `Label` enum says how a model line names its value: by its key in an object, or as an item of an array.
#[derive(Clone, Copy)]
enum Label<'a> {
	Key(&'a str),
	Item,
}

/// Prints JSON as indented `key: value` and `- item` lines, summarizing long arrays of numbers.
fn json_lines(value: &Json, label: Label<'_>, indent: usize, lines: &mut Vec<String>) {
	if lines.len() >= MAX_MODEL_LINES {
		if lines.len() == MAX_MODEL_LINES {
			lines.push(format!("{:width$}…", "", width = indent * 2));
		}
		return;
	}
	// Each line is built in one string.
	let mut line = format!("{:width$}", "", width = indent * 2);
	match label {
		Label::Key(key) => {
			line.push_str(key);
			line.push_str(": ");
		}
		Label::Item => line.push_str("- "),
	}
	match value {
		Json::Object(object) => {
			if object.is_empty() {
				line.push_str("{}");
			}
			lines.push(line);
			for (key, value) in object {
				json_lines(value, Label::Key(key), indent + 1, lines);
			}
		}
		Json::Array(array) if array.iter().all(Json::is_number) => {
			let _ = if array.len() <= MAX_INLINE_NUMBERS {
				write!(line, "{value}")
			} else {
				write!(line, "[{} numbers]", array.len())
			};
			lines.push(line);
		}
		Json::Array(array) => {
			let _ = write!(line, "[{}]", array.len());
			lines.push(line);
			for item in array {
				json_lines(item, Label::Item, indent + 1, lines);
			}
		}
		scalar => {
			let _ = write!(line, "{scalar}");
			lines.push(line);
		}
	}
}

/// Lists the whole store's usage for the usage pane.
fn usage_lines(catalog: &mut Catalog) -> Vec<Line<'static>> {
	let classes = view(
		catalog,
		&Filter::default(),
		Some(&Field::Class),
		&Sort {
			field: Field::Stored,
			reversed: false,
		},
	);
	let catalog = &*catalog;
	let usage = &catalog.usage;
	let metadata = catalog
		.rows
		.iter()
		.filter_map(|row| row.resource.as_ref())
		.map(|resource| resource.resource().len() as u64)
		.sum::<u64>();
	let mut lines = vec![
		Line::from(format!(
			"database {} · model metadata {}",
			size(usage.database_size),
			size(metadata)
		)),
		Line::default(),
		Line::from(format!("{:<20} {:>8} {:>12} {:>12}", "class", "count", "decoded", "stored")).fg(Color::Cyan),
	];
	// Failed bakes have no class, and they store nothing.
	let classes = classes.iter().filter(|group| group.name.as_deref() != Some(NO_VALUE_GROUP));
	// Classes come largest stored first, and each bar is relative to the largest.
	let largest = classes.clone().next().map_or(0, |class| class.stored);
	for class in classes {
		lines.push(Line::from(format!(
			"{:<20} {:>8} {:>12} {:>12} {}",
			class.name.as_deref().unwrap_or_default(),
			class.rows.len(),
			size(class.size),
			size(class.stored),
			bar(share(class.stored, largest), 12)
		)));
	}

	let (duplicate_groups, wasted) = catalog.duplicate_waste();
	lines.push(Line::default());
	lines.push(Line::from(format!(
		"duplicates: {} of identical payloads, {} wasted",
		plural(duplicate_groups, "group"),
		size(wasted)
	)));
	match usage.pack {
		Some(pack) => lines.push(Line::from(format!(
			"pack: {} · {} used · {} free in {} · largest free {} · fragmented: {}",
			size(pack.file_size),
			size(pack.file_size - pack.free_bytes),
			size(pack.free_bytes),
			plural(pack.free_slot_count, "range"),
			size(pack.largest_free_slot),
			if pack.is_excessive() { "yes" } else { "no" }
		))),
		None => {
			let (count, bytes) = catalog.orphaned_payloads();
			lines.push(Line::from(format!(
				"orphaned payloads: {}, {}",
				plural(count, "file"),
				size(bytes)
			)));
		}
	}
	lines.push(Line::from(format!(
		"staging files: {}, {}",
		plural(usage.staging_files.0, "file"),
		size(usage.staging_files.1)
	)));
	lines.push(Line::from(format!("bakes: {}", bake_summary(catalog))));
	lines
}

fn size(bytes: u64) -> String {
	utils::ByteSize(bytes).to_string()
}

/// Draws a share from 0 to 1 as a bar of full blocks.
fn bar(share: f64, width: usize) -> String {
	"█".repeat((share.clamp(0.0, 1.0) * width as f64).round() as usize)
}
