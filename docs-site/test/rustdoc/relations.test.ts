/**
 * Black-box tests of the API model → relation groups and graph seam, on the model of
 * `test/fixtures/rustdoc-sample`.
 */
import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { buildModel } from '@/lib/rustdoc/build-model';
import type { Crate } from '@/lib/rustdoc/format';
import { GRAPH_NODE_HEIGHT, relationGraph, relationGroups } from '@/lib/rust-api/relations';

const model = buildModel(
	JSON.parse(readFileSync(new URL('../fixtures/rustdoc-sample/rustdoc.json', import.meta.url), 'utf8')) as Crate,
);
const groupsOf = (key: string) => relationGroups(model, model.items[key]);

describe('relation groups', () => {
	test('lists what depends on an item before what it depends on', () => {
		expect(groupsOf('sample::engine::Shape').map((group) => group.label)).toEqual(['Field of', 'Returned by', 'Implements']);
	});

	test('links a carried relation to the function or method that carries it', () => {
		const acceptedBy = groupsOf('sample::engine::Point').find((group) => group.label === 'Accepted by')!;
		expect(acceptedBy.entries).toEqual([
			{ label: 'shape_at', href: '/docs/api/latest/sample/engine#fn.shape_at', summary: 'Builds a shape at a point.' },
		]);
	});
});

describe('relation graph', () => {
	const item = model.items['sample::engine::Shape'];
	const graph = relationGraph(item, relationGroups(model, item))!;

	test('puts dependents left of the item and dependencies right of it, with one edge per node', () => {
		const side = (label: string) => graph.nodes.find((node) => node.label === label)!;
		expect(side('Scene').x + side('Scene').width).toBeLessThan(graph.center.x);
		expect(side('Area').x).toBeGreaterThan(graph.center.x + graph.center.width);
		expect(graph.edges).toHaveLength(graph.nodes.length);
	});

	test('keeps every node inside the drawing without overlaps', () => {
		for (const node of [...graph.nodes, graph.center]) {
			expect(node.x).toBeGreaterThanOrEqual(0);
			expect(node.y).toBeGreaterThanOrEqual(0);
			expect(node.x + node.width).toBeLessThanOrEqual(graph.width);
			expect(node.y + GRAPH_NODE_HEIGHT).toBeLessThanOrEqual(graph.height);
		}
		const sorted = graph.nodes.filter((node) => node.x === graph.nodes[0].x).sort((left, right) => left.y - right.y);
		for (let index = 1; index < sorted.length; index++) {
			expect(sorted[index].y).toBeGreaterThanOrEqual(sorted[index - 1].y + GRAPH_NODE_HEIGHT);
		}
	});

	const group = (length: number) => ({
		...groupsOf('sample::engine::Shape')[0],
		items: Array.from({ length }, () => model.items['sample::engine::Scene']),
	});

	test('collapses a group past four items into a box that counts the rest', () => {
		const collapsed = relationGraph(item, [group(8)])!;
		expect(collapsed.nodes.map((node) => node.label)).toEqual(['Scene', 'Scene', 'Scene', '+5 more']);
		expect(collapsed.nodes.at(-1)!.href).toBeUndefined();
	});

	test('draws at most ten boxes per column however many groups an item has', () => {
		const crowded = relationGraph(item, Array.from({ length: 6 }, () => group(8)))!;
		expect(crowded.nodes).toHaveLength(10);
		expect(crowded.labels).toHaveLength(6);
	});

	test('draws nothing for an item without relations', () => {
		expect(relationGraph(model.items['sample::engine::Hidden'], [])).toBeUndefined();
	});
});
