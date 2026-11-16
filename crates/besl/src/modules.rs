//! Loads the files of BESL modules into a parsed program and checks the rules modules share.
//!
//! BESL does no file I/O. A host that compiles a shader file reads its module files through a [`ModuleLoader`], which
//! asks for the files to read one module at a time, then places them into the parsed shader with [`attach_modules`]
//! and passes it to [`crate::lex`]. Underneath, [`pending_modules`] lists what a parsed file needs, [`module_files`]
//! names the files a module may live in, and [`attach_module`] places one parsed file.
//!
//! A module is either one of the shader's own, found from the shader's directory by its module path, or a module from
//! the asset root, which a `use ::...` path names and which a program holds once in a scope named `::`.
//!
//! The language rules are in the Modules section of the BESL language reference.

use crate::parser::{Node, Nodes};
use crate::{CompilationError, lexer::LexError};

/// The documentation page of the module rules that errors point to.
const MODULES_DOCS_PATH: &str = "reference/besl/language#modules";

/// The name of the scope that holds a program's modules from the asset root.
pub(crate) const ROOT_MODULES: &str = "::";

/// The `ModuleRequest` enum describes a module a parsed file needs and doesn't hold, so a host can find its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleRequest<'a> {
	/// A `name: mod;` declaration, at this module path within the file, such as `["lighting", "brdf"]` for `brdf: mod;`
	/// inside `lighting`. Its files are [`module_files`] of the file's own location and this path.
	Declared(Vec<&'a str>),
	/// A `use` path from the asset root, without its leading `::`, such as `byte_engine::layouts::color::Color`. Its
	/// module is the shortest start of the path that has a file; try them with [`module_files`], shortest first.
	Root(&'a str),
}

/// Returns what `file`, a root that [`crate::parse`] returns, needs and doesn't hold: the file modules it declares
/// without source, and the modules from the asset root its `use` declarations name.
///
/// Next, read one of the files [`module_files`] names for each request, parse it with [`crate::parse`], and attach it
/// with [`attach_module`].
pub(crate) fn pending_modules<'a>(file: &Node<'a>) -> Vec<ModuleRequest<'a>> {
	/// Adds the requests under `node`, whose module path is `path`.
	fn collect<'a>(node: &Node<'a>, path: &mut Vec<&'a str>, pending: &mut Vec<ModuleRequest<'a>>, loaded: &[&str]) {
		let Nodes::Scope { children, .. } = node.node() else {
			return;
		};
		for child in children {
			match child.node() {
				Nodes::FileModule { name } => {
					let mut module = path.clone();
					module.push(name);
					pending.push(ModuleRequest::Declared(module));
				}
				Nodes::Use { path: used, .. } => {
					if let Some(used) = used.strip_prefix("::")
						&& !loaded.iter().any(|module| names_module(used, module))
						&& !pending.contains(&ModuleRequest::Root(used))
					{
						pending.push(ModuleRequest::Root(used));
					}
				}
				Nodes::Scope { name, .. } => {
					path.push(name);
					collect(child, path, pending, loaded);
					path.pop();
				}
				_ => {}
			}
		}
	}

	let loaded = root_module_names(file);
	let mut pending = Vec::new();
	collect(file, &mut Vec::new(), &mut pending, &loaded);
	pending
}

/// Returns the two files the module at `path` may live in. Exactly one of them must exist.
///
/// With no `root`, `path` is a module of the shader and the files are relative to the shader's directory: `a/b.besl`
/// and `a/b/mod.besl` for `["a", "b"]`. With a `root`, such as `byte_engine::layouts::color`, `path` is a module inside
/// that module from the asset root, and the files are relative to the asset root:
/// `byte_engine/layouts/color/x.besl` for `["x"]`, or `byte_engine/layouts/color.besl` for no `path`.
pub(crate) fn module_files(root: Option<&str>, path: &[&str]) -> [String; 2] {
	let base = root
		.map(|root| root.split("::"))
		.into_iter()
		.flatten()
		.chain(path.iter().copied())
		.collect::<Vec<_>>()
		.join("/");
	[format!("{base}.besl"), format!("{base}/mod.besl")]
}

/// Places the parsed file `module` in `program`.
///
/// With no `root`, the file is the shader's module at `path`, in place of the `name: mod;` declaration it belongs to.
/// With a `root` and no `path`, the file is the module from the asset root `root` names, such as
/// `byte_engine::layouts::color`. With both, it is the module at `path` inside that one. `module` is the root that
/// [`crate::parse`] returns for the file, and it can't declare `main`.
///
/// Next, call [`pending_modules`] on the attached file: it may need modules of its own.
pub(crate) fn attach_module<'a>(
	program: &mut Node<'a>,
	root: Option<&'a str>,
	path: &[&'a str],
	mut module: Node<'a>,
) -> Result<(), CompilationError> {
	let invalid = |message: String| CompilationError::Lex(LexError::invalid(message));
	let display = root
		.map(|root| format!("::{root}"))
		.into_iter()
		.chain(path.iter().map(|segment| segment.to_string()))
		.collect::<Vec<_>>()
		.join("::");

	let Nodes::Scope { children, .. } = module.node_mut() else {
		return Err(invalid(format!(
			"The source of module `{display}` is not a parsed file. The most likely cause is a node that `besl::parse` did not return. Pass the root it returns."
		)));
	};
	check_module_declarations(&display, children).map_err(invalid)?;
	let children = std::mem::take(children);

	let mut scope = program;
	if let Some(root) = root {
		scope = root_modules_mut(scope);
		if path.is_empty() {
			if scope.get_mut(root).is_some() {
				return Err(invalid(format!(
					"Module `{display}` is already attached. The most likely cause is a host that attaches a module from the asset root once per `use`. Attach it once, and skip the requests `pending_modules` no longer returns."
				)));
			}
			scope.add(vec![Node::scope(root, children)]);
			return Ok(());
		}
		scope = scope.get_mut(root).ok_or_else(|| {
			invalid(format!(
				"Module `::{root}` isn't attached. The most likely cause is a module inside it attached before it. Attach `::{root}` first."
			))
		})?;
	}

	let Some((name, parents)) = path.split_last() else {
		return Err(invalid(
			"Module path is empty. The most likely cause is a path that does not come from `pending_modules`. Pass one of the paths it returns.".to_string(),
		));
	};
	for parent in parents {
		scope = scope.get_mut(parent).filter(|node| matches!(node.node(), Nodes::Scope { .. })).ok_or_else(|| {
			invalid(format!(
				"Module `{display}` has no parent module `{parent}`. The most likely cause is a path that does not come from `pending_modules`. Pass one of the paths it returns."
			))
		})?;
	}

	let declaration = scope
		.get_mut(name)
		.filter(|node| matches!(node.node(), Nodes::FileModule { .. }))
		.ok_or_else(|| {
			invalid(format!(
				"Module `{display}` is not a pending file module. The most likely cause is that its file was already attached, or that no `{name}: mod;` declares it. Attach each path `pending_modules` returns once."
			))
		})?;

	*declaration = Node::scope(name_of(declaration), children);
	Ok(())
}

/// Returns the scope of `program`'s modules from the asset root, creating it first among the program's declarations so
/// they link before the shader's own.
fn root_modules_mut<'p, 'a>(program: &'p mut Node<'a>) -> &'p mut Node<'a> {
	let Nodes::Scope { children, .. } = program.node_mut() else {
		unreachable!("A program root is always a scope");
	};
	if !children.first().is_some_and(|child| child.name() == Some(ROOT_MODULES)) {
		children.insert(0, Node::scope(ROOT_MODULES, Vec::new()));
	}
	&mut children[0]
}

/// Moves the modules from the asset root that `program` declares as `::path: mod { ... }`, each parsed into a scope of
/// its own, into the one scope that holds them first in the program, where [`attach_modules`] places loaded ones.
pub(crate) fn gather_root_modules(program: &mut Node<'_>) {
	let Nodes::Scope { children, .. } = program.node_mut() else {
		return;
	};
	if children.iter().filter(|child| child.name() == Some(ROOT_MODULES)).count() < 2
		&& children.first().is_none_or(|child| child.name() == Some(ROOT_MODULES))
	{
		return;
	}
	let mut modules = Vec::new();
	children.retain_mut(|child| match child.node_mut() {
		Nodes::Scope { name, children } if *name == ROOT_MODULES => {
			modules.append(children);
			false
		}
		_ => true,
	});
	if !modules.is_empty() {
		children.insert(0, Node::scope(ROOT_MODULES, modules));
	}
}

/// Returns the names of the modules from the asset root that `program` holds.
fn root_module_names<'a>(program: &Node<'a>) -> Vec<&'a str> {
	let Nodes::Scope { children, .. } = program.node() else {
		return Vec::new();
	};
	children
		.iter()
		.find(|child| child.name() == Some(ROOT_MODULES))
		.map(|modules| match modules.node() {
			Nodes::Scope { children, .. } => children.iter().filter_map(Node::name).collect(),
			_ => Vec::new(),
		})
		.unwrap_or_default()
}

/// Reports whether the path from the asset root `path` starts with the module `module`, or names it.
fn names_module(path: &str, module: &str) -> bool {
	path.strip_prefix(module)
		.is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

/// Orders `program`'s modules from the asset root so each one links after the modules from the asset root it `use`s.
pub(crate) fn order_root_modules(program: &mut Node<'_>) -> Result<(), LexError> {
	/// Adds the `use ::...` paths under `node` to `paths`.
	fn root_uses<'a>(node: &Node<'a>, paths: &mut Vec<&'a str>) {
		let Nodes::Scope { children, .. } = node.node() else {
			return;
		};
		for child in children {
			match child.node() {
				Nodes::Use { path, .. } => paths.extend(path.strip_prefix("::")),
				Nodes::Scope { .. } => root_uses(child, paths),
				_ => {}
			}
		}
	}

	let Nodes::Scope { children, .. } = program.node_mut() else {
		return Ok(());
	};
	let Some(Nodes::Scope { children: modules, .. }) = children
		.iter_mut()
		.find(|child| child.name() == Some(ROOT_MODULES))
		.map(Node::node_mut)
	else {
		return Ok(());
	};

	// The modules each module needs, by index. Index the names so a `use` path finds the modules it starts with
	// through one lookup per `::` prefix instead of a scan of every module.
	let names = modules
		.iter()
		.map(|module| module.name().unwrap_or_default())
		.collect::<Vec<_>>();
	let mut modules_by_name = std::collections::HashMap::<&str, Vec<usize>>::with_capacity(names.len());
	for (index, name) in names.iter().enumerate() {
		modules_by_name.entry(*name).or_default().push(index);
	}
	let needs = modules
		.iter()
		.enumerate()
		.map(|(index, module)| {
			let mut paths = Vec::new();
			root_uses(module, &mut paths);
			let mut needs = Vec::new();
			for path in &paths {
				// A module is needed when its name is the whole path or a `::`-prefix of it.
				let mut end = path.len();
				loop {
					if let Some(indices) = modules_by_name.get(&path[..end]) {
						needs.extend(indices.iter().copied().filter(|&other| other != index));
					}
					match path[..end].rfind("::") {
						Some(separator) => end = separator,
						None => break,
					}
				}
			}
			needs.sort_unstable();
			needs.dedup();
			needs
		})
		.collect::<Vec<_>>();

	// Kahn's algorithm, keeping the attach order among modules that don't depend on each other. A min-heap of ready
	// indices takes the lowest index first, as scanning the list would, and each module is placed once.
	let mut dependents = vec![Vec::new(); names.len()];
	let mut remaining = vec![0usize; names.len()];
	for (index, module_needs) in needs.iter().enumerate() {
		remaining[index] = module_needs.len();
		for &need in module_needs {
			dependents[need].push(index);
		}
	}
	let mut ready = std::collections::BinaryHeap::new();
	for (index, &pending) in remaining.iter().enumerate() {
		if pending == 0 {
			ready.push(std::cmp::Reverse(index));
		}
	}
	let mut order = Vec::with_capacity(names.len());
	while let Some(std::cmp::Reverse(next)) = ready.pop() {
		order.push(next);
		for &dependent in &dependents[next] {
			remaining[dependent] -= 1;
			if remaining[dependent] == 0 {
				ready.push(std::cmp::Reverse(dependent));
			}
		}
	}
	if order.len() < names.len() {
		let cycle = (0..names.len())
			.filter(|&index| remaining[index] > 0)
			.map(|index| format!("`::{}`", names[index]))
			.collect::<Vec<_>>()
			.join(", ");
		return Err(LexError::invalid(format!(
			"Modules {cycle} name each other. The most likely cause is two shared modules that each `use` the other, but each one links after the modules it names. Move what both need into a third module. See {}.",
			crate::online_docs_url(MODULES_DOCS_PATH)
		)));
	}

	let mut taken = std::mem::take(modules).into_iter().map(Some).collect::<Vec<_>>();
	modules.extend(order.into_iter().filter_map(|index| taken[index].take()));
	Ok(())
}

/// The `ModuleLoader` struct walks a host through reading the module files one shader needs, so every host finds the
/// same files in the same order and reports the same errors.
///
/// Create it from the shader's source, then, while [`ModuleLoader::query`] returns files, read them and pass what you
/// read to [`ModuleLoader::answer`]. Read both files of every query, even when the first one exists: a module with two
/// files is an error, and a host that records what it reads, such as a bake, should depend on both. Finally, parse
/// the shader and place the modules with [`attach_modules`].
pub struct ModuleLoader {
	pending: std::collections::VecDeque<Pending>,
	modules: Vec<LoadedModule>,
}

/// One module the loader still has to find.
enum Pending {
	/// A file module at `path`, inside the module from the asset root `root`, or the shader's own when `root` is `None`.
	Declared { root: Option<String>, path: Vec<String> },
	/// A `use` path from the asset root, whose module is the shortest start of `segments` that has a file. `prefix` is
	/// how many segments the next query tries.
	Root { segments: Vec<String>, prefix: usize },
}

/// The `ModuleQuery` struct names the two files one module may live in, which the host reads next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleQuery {
	/// The two files, relative to the asset root when `from_root` is set, and to the shader's directory otherwise.
	pub files: [String; 2],
	pub from_root: bool,
}

/// The `LoadedModule` struct is one module file a [`ModuleLoader`] found, which [`attach_modules`] places in the
/// shader.
#[derive(Clone, Debug)]
pub struct LoadedModule {
	/// The module from the asset root the file is or is inside, such as `byte_engine::layouts::color`.
	pub root: Option<String>,
	/// The module's path inside `root`, or inside the shader when `root` is `None`.
	pub path: Vec<String>,
	/// The file it was read from, as its [`ModuleQuery`] named it.
	pub file: String,
	pub source: String,
}

impl ModuleLoader {
	/// Starts loading the modules `source`, a shader file, needs.
	pub fn new(source: &str) -> Result<Self, CompilationError> {
		let mut loader = Self {
			pending: std::collections::VecDeque::new(),
			modules: Vec::new(),
		};
		loader.enqueue(None, &[], pending_modules(&crate::parse(source)?));
		Ok(loader)
	}

	/// Returns the two files to read for the next module, or `None` when every module is loaded.
	pub fn query(&mut self) -> Option<ModuleQuery> {
		// A module from the asset root another request loaded meanwhile needs no more reading.
		while let Some(Pending::Root { segments, .. }) = self.pending.front()
			&& self.covers(&segments.join("::"))
		{
			self.pending.pop_front();
		}

		Some(match self.pending.front()? {
			Pending::Declared { root, path } => ModuleQuery {
				files: module_files(root.as_deref(), &path.iter().map(String::as_str).collect::<Vec<_>>()),
				from_root: root.is_some(),
			},
			Pending::Root { segments, prefix } => ModuleQuery {
				files: module_files(Some(&segments[..*prefix].join("::")), &[]),
				from_root: true,
			},
		})
	}

	/// Takes the sources of the files the last [`ModuleLoader::query`] named, `None` for a file that doesn't exist.
	pub fn answer(&mut self, sources: [Option<String>; 2]) -> Result<(), CompilationError> {
		let invalid = |message: String| CompilationError::Lex(LexError::invalid(message));
		let query = self.query().ok_or_else(|| {
			invalid("Module files were given with no module to load. The most likely cause is a host that answers without a query. Answer once per `ModuleLoader::query`.".to_string())
		})?;
		let pending = self.pending.front_mut().expect("A query means a module is pending");

		let [first, second] = sources;
		let (file, source) = match (first, second) {
			(Some(_), Some(_)) => {
				return Err(invalid(format!(
					"Module `{}` has two files, '{}' and '{}'. The most likely cause is a module moved into its own directory while the old file stayed. Delete one of them. See {}.",
					describe(pending),
					query.files[0],
					query.files[1],
					crate::online_docs_url(MODULES_DOCS_PATH)
				)));
			}
			(Some(source), None) => (query.files[0].clone(), source),
			(None, Some(source)) => (query.files[1].clone(), source),
			(None, None) => {
				// A path from the asset root tries a longer start next.
				if let Pending::Root { segments, prefix } = pending
					&& *prefix < segments.len()
				{
					*prefix += 1;
					return Ok(());
				}
				return Err(invalid(format!(
					"Module `{}` has no file. The most likely cause is a misspelled module name, or a file in another directory. Create '{}' or '{}'. See {}.",
					describe(pending),
					query.files[0],
					query.files[1],
					crate::online_docs_url(MODULES_DOCS_PATH)
				)));
			}
		};

		let (root, path) = match self.pending.pop_front().expect("A query means a module is pending") {
			Pending::Declared { root, path } => (root, path),
			Pending::Root { segments, prefix } => (Some(segments[..prefix].join("::")), Vec::new()),
		};

		let parsed = crate::parse(&source).map_err(|error| {
			invalid(format!(
				"Module file '{file}' did not parse ({error:?}). The most likely cause is invalid BESL syntax in the module."
			))
		})?;
		let requests = pending_modules(&parsed);
		self.enqueue(root.as_deref(), &path, requests);

		self.modules.push(LoadedModule {
			root,
			path,
			file,
			source,
		});
		Ok(())
	}

	/// Returns the modules found, parents before the modules they declare.
	pub fn into_modules(self) -> Vec<LoadedModule> {
		self.modules
	}

	/// Finds every module file `source`, a shader file, needs, reading each through `read`, which returns a file's
	/// source by the path a [`ModuleQuery`] names, or `None` when the file doesn't exist.
	///
	/// Use it where files can be read without waiting, such as from embedded sources or in tests; a host that reads
	/// files asynchronously drives [`Self::query`] and [`Self::answer`] itself. Next, pass the modules to
	/// [`attach_modules`].
	pub fn load(source: &str, mut read: impl FnMut(&str) -> Option<String>) -> Result<Vec<LoadedModule>, CompilationError> {
		let mut loader = Self::new(source)?;
		while let Some(query) = loader.query() {
			let answer = query.files.each_ref().map(|file| read(file));
			loader.answer(answer)?;
		}
		Ok(loader.into_modules())
	}

	/// Queues the requests of a file at `path` inside the module from the asset root `root`.
	fn enqueue(&mut self, root: Option<&str>, path: &[String], requests: Vec<ModuleRequest<'_>>) {
		for request in requests {
			match request {
				ModuleRequest::Declared(declared) => self.pending.push_back(Pending::Declared {
					root: root.map(str::to_string),
					path: path.iter().cloned().chain(declared.into_iter().map(str::to_string)).collect(),
				}),
				ModuleRequest::Root(used) => {
					let queued = self.pending.iter().any(
						|pending| matches!(pending, Pending::Root { segments, .. } if names_module(used, &segments.join("::"))),
					);
					if !queued && !self.covers(used) {
						self.pending.push_back(Pending::Root {
							segments: used.split("::").map(str::to_string).collect(),
							prefix: 1,
						});
					}
				}
			}
		}
	}

	/// Reports whether a loaded module from the asset root starts the `use` path `used`.
	fn covers(&self, used: &str) -> bool {
		self.modules
			.iter()
			.any(|module| module.path.is_empty() && module.root.as_deref().is_some_and(|root| names_module(used, root)))
	}
}

/// Describes the module `pending` asks for, for errors.
fn describe(pending: &Pending) -> String {
	match pending {
		Pending::Declared { root, path } => root
			.iter()
			.map(|root| format!("::{root}"))
			.chain(path.iter().cloned())
			.collect::<Vec<_>>()
			.join("::"),
		Pending::Root { segments, .. } => format!("::{}", segments.join("::")),
	}
}

/// Places every module of `modules`, as a [`ModuleLoader`] found them, in `program`, the shader file's parsed root.
///
/// Next, pass `program` to [`crate::lex`].
pub fn attach_modules<'a>(program: &mut Node<'a>, modules: &'a [LoadedModule]) -> Result<(), CompilationError> {
	for module in modules {
		let path = module.path.iter().map(String::as_str).collect::<Vec<_>>();
		let parsed = crate::parse(&module.source)?;
		attach_module(program, module.root.as_deref(), &path, parsed)?;
	}
	Ok(())
}

/// Returns the name a module declaration declares.
fn name_of<'a>(declaration: &Node<'a>) -> &'a str {
	declaration.name().expect("A module declaration always has a name")
}

/// Checks that the module `module` doesn't declare `main`, which only the shader file declares.
pub(crate) fn check_module_declarations(module: &str, declarations: &[Node<'_>]) -> Result<(), String> {
	if declarations
		.iter()
		.any(|declaration| matches!(declaration.node(), Nodes::Function { name: "main", .. }))
	{
		return Err(format!(
			"Module `{module}` declares `main`. The most likely cause is a shader file declared as a module. Keep `main` in the shader file, and move only its helpers into the module. See {}.",
			crate::online_docs_url(MODULES_DOCS_PATH)
		));
	}
	Ok(())
}

/// Checks that no two modules of `program` declare the same struct or constant name, or the same function name with
/// the same parameter types.
///
/// Backends emit every declaration by its own name, so two such declarations would collide in the generated code.
/// Functions that differ in their parameter types emit as overloads, which every backend accepts.
pub(crate) fn check_unique_names(program: &Node<'_>) -> Result<(), LexError> {
	/// The `Modules` struct records, while one program is walked, the path of every module and the declarations each
	/// name refers to.
	struct Modules<'p, 'a> {
		paths: Vec<Vec<&'a str>>,
		declarations: std::collections::HashMap<&'a str, Vec<(usize, &'p Node<'a>)>>,
	}

	/// Returns the type of a function parameter.
	fn parameter_type<'p, 'a>(parameter: &'p Node<'a>) -> Option<&'p crate::parser::TypeName<'a>> {
		match parameter.node() {
			Nodes::Parameter { r#type, .. } => Some(r#type),
			_ => None,
		}
	}

	/// Reports whether `a` and `b` collide when both are emitted by their shared name.
	fn collide<'a>(a: &Node<'a>, b: &Node<'a>) -> bool {
		match (a.node(), b.node()) {
			(Nodes::Function { params: a, .. }, Nodes::Function { params: b, .. }) => {
				a.iter().map(parameter_type).eq(b.iter().map(parameter_type))
			}
			_ => true,
		}
	}

	/// Records the declarations in `node`, the module at `path`, and checks them against every other module's.
	fn visit<'p, 'a>(node: &'p Node<'a>, path: &mut Vec<&'a str>, modules: &mut Modules<'p, 'a>) -> Result<(), LexError> {
		let Nodes::Scope { children, .. } = node.node() else {
			return Ok(());
		};
		let module = modules.paths.len();
		modules.paths.push(path.clone());
		for child in children {
			match child.node() {
				Nodes::Scope { name, .. } => {
					path.push(name);
					visit(child, path, modules)?;
					path.pop();
				}
				Nodes::Struct { name, .. } | Nodes::Const { name, .. } | Nodes::Function { name, .. } => {
					let declarations = modules.declarations.entry(name).or_default();
					if let Some((owner, _)) = declarations
						.iter()
						.find(|(owner, other)| *owner != module && collide(child, other))
					{
						let describe = |path: &[&str]| match path {
							[] => "the shader".to_string(),
							// A module from the asset root sits under the `::` scope, and its name is already a path.
							[ROOT_MODULES, rest @ ..] => format!("module `::{}`", rest.join("::")),
							path => format!("module `{}`", path.join("::")),
						};
						return Err(LexError::invalid(format!(
							"`{name}` is declared in both {} and {}. The most likely cause is two modules that each declare a helper with the same name. Rename one of them, because backends emit every declaration by its own name. See {}.",
							describe(&modules.paths[*owner]),
							describe(path),
							crate::online_docs_url(MODULES_DOCS_PATH)
						)));
					}
					declarations.push((module, child));
				}
				_ => {}
			}
		}
		Ok(())
	}

	// Only programs with modules can declare a name in two modules.
	let Nodes::Scope { children, .. } = program.node() else {
		return Ok(());
	};
	if !children
		.iter()
		.any(|child| matches!(child.node(), Nodes::Scope { name, .. } if *name != "MACRO"))
	{
		return Ok(());
	}

	let mut modules = Modules {
		paths: Vec::new(),
		declarations: std::collections::HashMap::new(),
	};
	visit(program, &mut Vec::new(), &mut modules)
}

/// Reports a `name: mod;` declaration whose file was never attached.
pub(crate) fn unattached_module(name: &str) -> LexError {
	LexError::invalid(format!(
		"Module `{name}` has no source. The most likely cause is that the program was compiled without loading its module files. Load them with `besl::pending_modules` and `besl::attach_module` before linking. See {}.",
		crate::online_docs_url(MODULES_DOCS_PATH)
	))
}

/// Reports whether `name` is reserved for a path root or the program root, so no module can take it.
pub(crate) fn is_reserved_module_name(name: &str) -> bool {
	matches!(name, "crate" | "super" | "self" | "root")
}
