import { GRAPH_NODE_HEIGHT, type GraphNode, type RelationGraphView } from '@/lib/rust-api/relations';

/**
 * Draws a laid-out relation graph as SVG. Each item box links to its page; the page item
 * sits in the middle, the items that depend on it on the left, and its dependencies on the right.
 */
export function RelationGraph({ graph, title }: { graph: RelationGraphView; title: string }) {
	return (
		<div className="be-rust-graph">
			<svg
				viewBox={`0 0 ${graph.width} ${graph.height}`}
				width={graph.width}
				height={graph.height}
				role="img"
				aria-label={title}
			>
				<defs>
					<marker id="be-rust-graph-arrow" viewBox="0 0 8 8" refX="8" refY="4" markerWidth="8" markerHeight="8" orient="auto">
						<path d="M0,0 L8,4 L0,8 z" className="be-rust-graph-arrow" />
					</marker>
				</defs>
				{graph.edges.map((edge, index) => (
					<path key={index} d={edge.path} className="be-rust-graph-edge" data-kind={edge.kind} markerEnd="url(#be-rust-graph-arrow)" />
				))}
				{graph.labels.map((label, index) => (
					<text key={index} x={label.x} y={label.y} className="be-rust-graph-label">
						{label.text}
					</text>
				))}
				{graph.nodes.map((node, index) => (
					<Node key={index} node={node} />
				))}
				<Node node={graph.center} current />
			</svg>
		</div>
	);
}

function Node({ node, current = false }: { node: GraphNode; current?: boolean }) {
	const box = (
		<g className="be-rust-graph-node" data-kind={node.kind} data-current={current || undefined}>
			<title>{node.title}</title>
			<rect x={node.x} y={node.y} width={node.width} height={GRAPH_NODE_HEIGHT} rx={6} />
			<text x={node.x + node.width / 2} y={node.y + GRAPH_NODE_HEIGHT / 2} textAnchor="middle" dominantBaseline="central">
				{node.label}
			</text>
		</g>
	);
	return node.href ? <a href={node.href}>{box}</a> : box;
}
