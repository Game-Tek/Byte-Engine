import type { ApiItem, ApiModel, ApiReference, ApiRelation, ApiRelationKind } from '@/lib/rustdoc/model';
import { describe } from './items';

/** Whether the item a group lists points at the page item (`incoming`) or the page item points at it (`outgoing`). */
export type RelationDirection = 'incoming' | 'outgoing';

/** How relation groups are titled, read with the listed item first for `incoming` and the page item first for `outgoing`. */
const LABELS: Record<ApiRelationKind, Record<RelationDirection, string>> = {
	field: { incoming: 'Field of', outgoing: 'Has fields of' },
	implements: { incoming: 'Implemented by', outgoing: 'Implements' },
	supertrait: { incoming: 'Required by', outgoing: 'Requires' },
	accepts: { incoming: 'Accepted by', outgoing: 'Accepts' },
	returns: { incoming: 'Returned by', outgoing: 'Returns' },
	mentions: { incoming: 'Mentioned by', outgoing: 'Mentions' },
};

/** The order of relation kinds in lists and in each graph column, strongest dependency first. */
const KIND_ORDER: ApiRelationKind[] = ['implements', 'supertrait', 'field', 'accepts', 'returns', 'mentions'];

/** The `RelationGroup` interface gathers the relations of one kind and direction, which item pages list and graph together. */
export interface RelationGroup {
	kind: ApiRelationKind;
	direction: RelationDirection;
	label: string;
	/** The related items, once each, which the graph draws as nodes. */
	items: ApiItem[];
	/** What a reader follows: the carrying function or method when there is one, otherwise the related item. */
	entries: ApiReference[];
}

const indexes = new WeakMap<ApiModel, Map<string, Record<RelationDirection, ApiRelation[]>>>();

/** Returns the relations that start or end at an item, indexing the model on first use. */
function relationsOf(model: ApiModel, key: string): Record<RelationDirection, ApiRelation[]> {
	let index = indexes.get(model);
	if (!index) {
		index = new Map();
		const at = (itemKey: string) => {
			let entry = index!.get(itemKey);
			if (!entry) index!.set(itemKey, (entry = { incoming: [], outgoing: [] }));
			return entry;
		};
		for (const relation of model.relations) {
			at(relation.from).outgoing.push(relation);
			at(relation.to).incoming.push(relation);
		}
		indexes.set(model, index);
	}
	return index.get(key) ?? { incoming: [], outgoing: [] };
}

/**
 * Groups the relations of an item by direction and kind, in reading order: what depends
 * on the item, then what the item depends on. Item pages and their Markdown versions list
 * these groups, and {@link relationGraph} draws them.
 */
export function relationGroups(model: ApiModel, item: ApiItem): RelationGroup[] {
	const relations = relationsOf(model, item.key);
	const groups: RelationGroup[] = [];
	for (const direction of ['incoming', 'outgoing'] as const) {
		for (const kind of KIND_ORDER) {
			const matching = relations[direction].filter((relation) => relation.kind === kind);
			if (matching.length === 0) continue;

			const items = new Map<string, ApiItem>();
			const entries = new Map<string, ApiReference>();
			for (const relation of matching) {
				const other = model.items[direction === 'incoming' ? relation.from : relation.to];
				if (!other) continue;
				items.set(other.key, other);
				const entry = relation.via ?? { label: other.name, href: other.route, summary: describe(other) };
				entries.set(entry.href, entry);
			}
			if (items.size === 0) continue;

			groups.push({
				kind,
				direction,
				label: LABELS[kind][direction],
				items: [...items.values()].sort((left, right) => left.name.localeCompare(right.name)),
				entries: [...entries.values()].sort((left, right) => left.label.localeCompare(right.label)),
			});
		}
	}
	return groups;
}

// -------------------------------------------------------------------------
// Graph layout

/** One box of a relation graph. A node without `href` stands for the related items the graph leaves out. */
export interface GraphNode {
	label: string;
	title: string;
	kind?: ApiItem['kind'];
	href?: string;
	x: number;
	y: number;
	width: number;
}

/** A group heading drawn above the first node of a group, at its left edge. */
export interface GraphLabel {
	text: string;
	x: number;
	y: number;
}

/** The `RelationGraphView` interface is a laid-out relation graph that an item page draws as SVG without further measuring. */
export interface RelationGraphView {
	width: number;
	height: number;
	center: GraphNode;
	nodes: GraphNode[];
	labels: GraphLabel[];
	/** Cubic Bézier paths, each from a related node to the page item or from the page item to a related node. */
	edges: { path: string; kind: ApiRelationKind }[];
}

export const GRAPH_NODE_HEIGHT = 28;
/** The graph draws labels in a 12px monospace font; this is its advance width, which sizes the boxes. */
const CHARACTER_WIDTH = 7.25;
const NODE_PADDING = 12;
const NODE_GAP = 6;
/** The vertical room a group heading takes above its first node. */
const LABEL_HEIGHT = 22;
const GROUP_GAP = 10;
const COLUMN_GAP = 64;
const MARGIN = 4;
const MAX_LABEL_LENGTH = 24;
/** The most boxes a graph draws per group; the rest collapse into one "+N more" box. */
const MAX_GROUP_NODES = 4;
/** The most boxes a graph draws per column, which bounds its height whatever the item's relations. */
const MAX_COLUMN_NODES = 10;

/**
 * Lays out the relations of one item in three columns: the items that depend on it on
 * the left, the item in the middle, and the items it depends on on the right. Every edge
 * runs left to right, so the graph needs no crossing reduction, and each group stacks
 * under its heading. Returns `undefined` for an item without relations.
 */
export function relationGraph(item: ApiItem, groups: RelationGroup[]): RelationGraphView | undefined {
	if (groups.length === 0) return undefined;

	const columns = (['incoming', 'outgoing'] as const).map((direction) =>
		stackColumn(groups.filter((group) => group.direction === direction)),
	);
	const [left, right] = columns;
	const centerLabel = truncate(item.name);
	const centerWidth = boxWidth(centerLabel);
	const leftWidth = left.width;
	const centerX = MARGIN + (leftWidth > 0 ? leftWidth + COLUMN_GAP : 0);
	const rightX = centerX + centerWidth + (right.width > 0 ? COLUMN_GAP : 0);
	const height = Math.max(left.height, right.height, GRAPH_NODE_HEIGHT) + MARGIN * 2;

	const center: GraphNode = {
		label: centerLabel,
		title: item.key,
		kind: item.kind,
		x: centerX,
		y: (height - GRAPH_NODE_HEIGHT) / 2,
		width: centerWidth,
	};
	const centerMiddle = center.y + GRAPH_NODE_HEIGHT / 2;

	const nodes: GraphNode[] = [];
	const labels: GraphLabel[] = [];
	const edges: RelationGraphView['edges'] = [];
	for (const [column, x, direction] of [
		[left, MARGIN, 'incoming'],
		[right, rightX, 'outgoing'],
	] as const) {
		// Center each column against the taller one so short columns sit level with the page item.
		const offset = MARGIN + (height - MARGIN * 2 - column.height) / 2;
		for (const label of column.labels) labels.push({ ...label, x: x + label.x, y: label.y + offset });
		for (const { node, kind } of column.nodes) {
			const placed = { ...node, x: x + node.x, y: node.y + offset };
			nodes.push(placed);
			const middle = placed.y + GRAPH_NODE_HEIGHT / 2;
			edges.push({
				kind,
				path:
					direction === 'incoming'
						? curve(placed.x + placed.width, middle, center.x, centerMiddle)
						: curve(center.x + center.width, centerMiddle, placed.x, middle),
			});
		}
	}

	const width = Math.max(rightX + right.width, centerX + centerWidth) + MARGIN;
	return { width, height, center, nodes, labels, edges };
}

/**
 * Returns how many boxes each group may draw: up to {@link MAX_GROUP_NODES}, taking one
 * from the largest group until the column fits {@link MAX_COLUMN_NODES}. Every group keeps one box.
 */
function boxBudgets(groups: RelationGroup[]): number[] {
	const budgets = groups.map((group) => Math.min(group.items.length, MAX_GROUP_NODES));
	let total = budgets.reduce((sum, budget) => sum + budget, 0);
	while (total > MAX_COLUMN_NODES) {
		const largest = budgets.indexOf(Math.max(...budgets));
		if (budgets[largest] <= 1) break;
		budgets[largest]--;
		total--;
	}
	return budgets;
}

/** Stacks groups in one column, with column-relative positions, within the box budget of each group. */
function stackColumn(groups: RelationGroup[]) {
	const nodes: { node: GraphNode; kind: ApiRelationKind }[] = [];
	const labels: GraphLabel[] = [];
	const budgets = boxBudgets(groups);
	let y = 0;
	let width = 0;
	for (const [index, group] of groups.entries()) {
		if (y > 0) y += GROUP_GAP;
		labels.push({ text: group.label, x: 0, y: y + LABEL_HEIGHT - 8 });
		width = Math.max(width, group.label.length * CHARACTER_WIDTH);
		y += LABEL_HEIGHT;

		// A group over budget gives its last box to the "+N more" box.
		const budget = budgets[index];
		const shown = group.items.length > budget ? group.items.slice(0, budget - 1) : group.items;
		const boxes: GraphNode[] = shown.map((related) => ({
			label: truncate(related.name),
			title: related.key,
			kind: related.kind,
			href: related.route,
			x: 0,
			y: 0,
			width: boxWidth(truncate(related.name)),
		}));
		const hidden = group.items.length - shown.length;
		if (hidden > 0) {
			const label = `+${hidden} more`;
			boxes.push({ label, title: `${hidden} more, listed under ${group.label}`, x: 0, y: 0, width: boxWidth(label) });
		}
		for (const box of boxes) {
			nodes.push({ node: { ...box, y }, kind: group.kind });
			width = Math.max(width, box.width);
			y += GRAPH_NODE_HEIGHT + NODE_GAP;
		}
		y -= NODE_GAP;
	}
	return { nodes, labels, width, height: y };
}

function boxWidth(label: string): number {
	return Math.ceil(label.length * CHARACTER_WIDTH + NODE_PADDING * 2);
}

function truncate(label: string): string {
	return label.length > MAX_LABEL_LENGTH ? `${label.slice(0, MAX_LABEL_LENGTH - 1)}…` : label;
}

/** A horizontal S-curve between two points, with control points halfway across. */
function curve(fromX: number, fromY: number, toX: number, toY: number): string {
	const middle = (fromX + toX) / 2;
	return `M${fromX},${fromY} C${middle},${fromY} ${middle},${toY} ${toX},${toY}`;
}
