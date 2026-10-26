# Documentation

Every page in `docs` belongs to exactly one section. Choose the section by who reads the page and what they are asking, never by the system the page covers. The charter is `docs/develop/documentation.mdx` (the "Write documentation" page); keep this table and that page in agreement.

| Section | Directory | The reader asks | A page is | Never |
| --- | --- | --- | --- | --- |
| Introduction | `docs/*.mdx` | What is this, and where do I start? | A short orientation that sends the reader into a section | A task, a design note, or a definition |
| Use | `docs/use` | How do I make my application do this? | One task: the goal first, the steps, the code the reader writes, Rust API links at the end | Engine internals, design rationale, anything about the repository |
| Contribute | `docs/develop` | How do I work on the engine, and what must my change keep true? | A workflow page (build, test, bench, measure, publish, document) or a design note for one system: how it is built, why, the rules a change must keep, the tests that guard them | How to use a feature from an application; the complete definition of a format or language |
| Reference | `docs/reference` | What exactly is this, and what does it accept? | A specification of one thing: a language, format, model, unit, or limit, as definitions, tables, and grammar; what is accepted and what results | Code: no Rust, no file examples, no API calls, no account of how the engine does it inside; also no numbered steps, tutorials, or the reasons behind a design. A code block may show only the syntax of the language the page specifies (BESL) |
| API | `docs/api` | What does this item do? | Generated from rustdoc comments and `docs-site/openapi/inspector.json`; Git ignores `docs/api/latest` | Hand-written edits; change the doc comment or the schema instead |

Decide in this order and stop at the first yes: does the page tell someone how to change the engine or what a change must preserve (Contribute); does it tell someone how to do something from their application (Use); does it define something to look up (Reference); is it about one Rust item or endpoint (a doc comment or the OpenAPI schema). A page that answers two of these is two pages: keep the steps in Use or Contribute, move the definitions to Reference, and link them. Reference pages are specifications: when one needs an example, write the example in a Use page and link it; when one needs to explain a mechanism, write that in a Contribute design note and link it.

Run `bun run check:style` from `docs-site` after editing a page: it lints the pages with Vale against the Microsoft style package and the rules in `docs-site/styles/ByteEngine`. Fix what it reports instead of adding exceptions, unless the word is an engine term, which goes in `docs-site/styles/config/vocabularies/ByteEngine/accept.txt`.

Give every page `title`, `description`, and `icon` front matter. The description is one sentence that answers the section's question for that page; the site shows it in navigation and serves it to agents through `llms.txt`.

Use ISO 24495-1 plain-language principles for in-code comments and API documentation. Make the content relevant, findable, understandable, and usable for its intended developer.

Use Microsoft Writing Style Guide principles for user guides and documentation pages. Address the reader directly, lead with the reader's goal, use everyday words, keep the tone helpful, and make the next action clear.

Keep useful intra-code links in in-code and API documentation, even when strict application of ISO 24495-1 would simplify or remove them. Links to related types, traits, functions, methods, modules, and concepts help developers navigate the API.

For workflow-bearing APIs, include a short next-step suggestion that links to the API a developer usually calls or implements next. Add this guidance to modules, types, constructors, and transition methods where it helps developers connect subsystems. Do not add it to trivial fields or accessors.

Always write struct and trait documentation as:
The `<NAME>` struct/trait <PURPOSE>

Always write about the purpose of the struct/trait or why it exists instead of what it does.
Write usage-oriented documentation. How/where will this element be used, instead of this is what it does or how.

If a function is longer than a few lines, write a short description of what the function does.

# Errors and log messages

Write every error and every logged warning or error in three parts:

1. A succinct statement of what happened.
2. A sentence with the most likely cause.
3. A suggestion: the action that most likely fixes it, such as the parameter to change and to what.

When the problem has a documented recovery workflow, end with a direct link to the relevant online documentation page, built with `online_docs_url`. Do not link to a generic documentation landing page when a focused page exists.

# API Design

- Delete first. If the task can be accomplished without adding a new function, do so.
- Prefer composition over inheritance. Use traits to define shared behavior and structs to encapsulate data.
- Prefer pure functions. Just inputs and outputs.
- Keep mutability at higher call sites.
- Keep dependency injection in mind.
- Separate concerns by layers. E.G: If some feature requires keeping track of occupied and free spaces in a file, write one piece of code that keeps track of that without concerning itself with how to read or modify the file, and the another piece of code that takes the former function's result and applied that to the file. Don't coalesce concerns. 

# Working

- Delete first.
- If I ask you to defer any task, write that into the todo.md file
- Always leave comments for any non-trivial code
- Always prefer breaking APIs to ad-hoc changes. The app is not shipped yet. We can make all breaking changes.
- If modifying performance sensitive code, run any existing benchmarks and ensure they do not regress (or they improve) before and after your changes.

# Tests
- Use `cargo nextest run --workspace` as the default test command. Run documentation tests separately with `cargo test --doc --workspace`.
- Tests must exercise contiguous seams. (E.G: don't test material evaluation shaders correctly lower to MSL. MSL lowering is responsability of the BESL MSL shader generator. Material evaluation must only ensure their BESL shader correctly lexes, which is asserted with a BESL lexer test).
- Do not add tombstone tests whose only purpose is to assert that removed code, routes, fields, or features remain absent. Negative tests are appropriate when the failure or absence is itself a current API, security, or persistence contract.
- Test observable behavior and specifications. Write black-box tests that verify the expected behavior of the system. Never write white-box tests that verify internal implementation details. To assert internal implementation details add assertions to the implementation code itself.

# Best Practices

- Avoid allocations, especially transient ones.
