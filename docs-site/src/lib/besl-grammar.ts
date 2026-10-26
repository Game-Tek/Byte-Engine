import type { LanguageRegistration } from 'shiki';

/**
 * The `beslGrammar` TextMate grammar lets ```besl code blocks in the docs highlight as BESL. Shiki doesn't bundle
 * BESL, so `source.config.ts` registers it with the MDX code highlighter; without it every page fails to render.
 * Keep the keyword and type lists in step with `docs/reference/besl/language.mdx`.
 */
export const beslGrammar: LanguageRegistration = {
	name: 'besl',
	displayName: 'BESL',
	scopeName: 'source.besl',
	patterns: [
		{ include: '#comments' },
		{ include: '#declarations' },
		{ include: '#keywords' },
		{ include: '#types' },
		{ include: '#literals' },
		{ include: '#calls' },
		{ include: '#operators' },
	],
	repository: {
		comments: {
			patterns: [
				{ name: 'comment.line.double-slash.besl', match: '//.*$' },
				{ name: 'comment.block.besl', begin: '/\\*', end: '\\*/' },
			],
		},
		// `name: fn`, `Name: struct`, `NAME: const` and the other `name: kind` module declarations.
		declarations: {
			patterns: [
				{
					match: '\\b([A-Za-z_][A-Za-z0-9_]*)\\s*:\\s*(fn)\\b',
					captures: {
						1: { name: 'entity.name.function.besl' },
						2: { name: 'storage.type.function.besl' },
					},
				},
				{
					match: '\\b([A-Za-z_][A-Za-z0-9_]*)\\s*:\\s*(struct)\\b',
					captures: {
						1: { name: 'entity.name.type.struct.besl' },
						2: { name: 'storage.type.struct.besl' },
					},
				},
				{
					match: '\\b([A-Za-z_][A-Za-z0-9_]*)\\s*:\\s*(const|descriptor|push_constant|workgroup|macro)\\b',
					captures: {
						1: { name: 'variable.other.declaration.besl' },
						2: { name: 'storage.modifier.besl' },
					},
				},
			],
		},
		keywords: {
			patterns: [
				{ name: 'keyword.control.besl', match: '\\b(if|else|for|return|break|continue|discard)\\b' },
				{ name: 'storage.type.besl', match: '\\b(let|fn|struct|const|descriptor|push_constant|workgroup|macro)\\b' },
			],
		},
		types: {
			patterns: [
				{
					name: 'support.type.primitive.besl',
					match:
						'\\b(void|bool|u8|u16|u32|i32|f16|f32|atomicu32|atomici32|vec[234](?:u16|f16|u|i|f)|mat4x3f|mat4f)\\b',
				},
				// User structs and resource handle types are written in PascalCase.
				{ name: 'entity.name.type.besl', match: '\\b[A-Z][a-z0-9]+[A-Za-z0-9_]*\\b' },
			],
		},
		literals: {
			patterns: [
				{ name: 'constant.language.boolean.besl', match: '\\b(true|false)\\b' },
				{ name: 'constant.numeric.besl', match: '\\b(0x[0-9A-Fa-f_]+|[0-9][0-9_]*(\\.[0-9_]+)?([eE][+-]?[0-9]+)?)\\b' },
				{ name: 'string.quoted.double.besl', begin: '"', end: '"' },
			],
		},
		calls: {
			patterns: [
				{
					match: '\\b([a-z_][A-Za-z0-9_]*)\\s*(?=\\()',
					captures: { 1: { name: 'entity.name.function.call.besl' } },
				},
			],
		},
		operators: {
			patterns: [
				{ name: 'keyword.operator.arrow.besl', match: '->' },
				{ name: 'keyword.operator.besl', match: '<<|>>|==|!=|<=|>=|&&|\\|\\||[-+*/%&|^!<>=]' },
			],
		},
	},
};
