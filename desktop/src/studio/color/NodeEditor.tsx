import { memo, useMemo } from 'react';
import { useWords } from '../../app/locale';
import type { Grade } from '../engine';
import type { GradeLevel } from './clips';
import {
  END_RADIUS, isComplete, layoutGrade, linkEnds, linkName, linkPath, NODE_HEIGHT, NODE_WIDTH, numberText, PORT_INSET, shownLabel,
  inputPoint, inputsOf, outputPoint, hasOutput, type PlacedNode,
} from './graph';

/** The badge of a shot match on a node, a small grid of dots as in p. 3048's picture. */
export function MatchedMark({ x, y, title }: { x: number; y: number; title: string }) {
  const dots = [0, 1, 2].flatMap(row => [0, 1, 2].map(column => [x + column * 3.5, y + row * 3.5] as const));
  return <g className="color-node-matched" data-test="node-matched"><title>{title}</title>
    {dots.map(([cx, cy]) => <circle key={`${cx}-${cy}`} cx={cx} cy={cy} r={1.1} />)}</g>;
}

/** The ports of a node: inputs on its left side, outputs on its right; the picture's green, the key's blue (p. 3311). */
function Ports({ placed }: { placed: PlacedNode }) {
  const { node } = placed;
  const ports = [];
  for (const kind of ['rgb', 'key'] as const) {
    for (let input = 0; input < inputsOf(node, kind); input++) {
      const at = inputPoint(placed, kind, input);
      ports.push(<path key={`in-${kind}-${input}`} className="color-port" data-kind={kind} data-port={`in:${kind}:${input}`}
        d={`M ${at.x - 1} ${at.y - 5} L ${at.x + 6} ${at.y} L ${at.x - 1} ${at.y + 5} Z`} />);
    }
    if (hasOutput(node, kind)) {
      const at = outputPoint(placed, kind);
      ports.push(<rect key={`out-${kind}`} className="color-port" data-kind={kind} data-port={`out:${kind}`} x={at.x - 5} y={at.y - 4} width={7} height={8} />);
    }
  }
  return <>{ports}</>;
}

/**
 * The node editor's graph, as DaVinci Resolve draws it (Reference Manual 20, pp. 3309–3313): the
 * source on the left, each node a card with its number (01, 02 … in the order of the grade) and
 * its label above it, links of the picture and of the key drawn apart, and the output on the right.
 * Shown, not edited: a press makes a node the current one, which is orange, and nothing else
 * changes (T16 edits). A grade that a node is not connected in colours nothing, and says so.
 */
export const GradeGraph = memo(function GradeGraph({ grade, current, onCurrent }: {
  grade: Grade;
  /** The node chosen, drawn orange, and told when another is pressed. */
  current: string | null;
  onCurrent: (node: string) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const layout = useMemo(() => layoutGrade(grade), [grade]);
  return <svg className="color-graph" width={layout.width} height={layout.height} viewBox={`0 0 ${layout.width} ${layout.height}`}
    role="group" aria-label={labels.nodeEditor} data-test="grade-graph">
    <g className="color-links">
      {layout.links.map((link) => {
        const ends = linkEnds(layout, link);
        return ends && <path key={linkName(link)} className="color-link" data-kind={link.kind} data-link={linkName(link)} d={linkPath(ends.from, ends.to)} />;
      })}
    </g>
    <g className="color-end" data-test="node-source">
      <title>{labels.nodeSource}</title>
      <circle cx={layout.source.x} cy={layout.source.y} r={END_RADIUS} />
    </g>
    <g className="color-end" data-end="output" data-test="node-output">
      <title>{labels.nodeOutput}</title>
      <circle cx={layout.output.x} cy={layout.output.y} r={END_RADIUS} />
    </g>
    {layout.nodes.map((placed) => {
      const { node, number, x, y } = placed;
      const name = node.kind === 'corrector' ? node.label : node.label || labels.kinds[node.kind];
      const matched = node.kind === 'corrector' && node.params.shot_match != null;
      return <g key={node.id} className="color-node" data-node={node.id} data-number={number} data-kind={node.kind}
        data-enabled={node.enabled} data-current={node.id === current || undefined} onPointerDown={(event) => { if (event.button === 0) onCurrent(node.id); }}>
        <title>{[`${numberText(number)}`, name, labels.kinds[node.kind], node.enabled ? '' : labels.disabled, matched ? labels.matched : ''].filter(Boolean).join(' · ')}</title>
        {name && <text className="color-node-label" x={x + NODE_WIDTH / 2} y={y - 6} textAnchor="middle" data-test="node-label">{shownLabel(name)}</text>}
        <rect className="color-node-card" x={x} y={y} width={NODE_WIDTH} height={NODE_HEIGHT} rx={5} />
        <text className="color-node-number" x={x + 9} y={y + PORT_INSET + 4} data-test="node-number">{numberText(number)}</text>
        {node.kind !== 'corrector' && <text className="color-node-kind" x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT / 2 + 4} textAnchor="middle">{labels.kinds[node.kind]}</text>}
        {matched && <MatchedMark x={x + NODE_WIDTH - 20} y={y + NODE_HEIGHT - 13} title={labels.matched} />}
        <Ports placed={placed} />
      </g>;
    })}
  </svg>;
});

/**
 * The node editor: which of the grades of the current clip it shows (Clip or Timeline, and for a
 * clip in a group its Group Pre-Clip and Group Post-Clip, p. 3303, chosen at its top right, as a
 * list or by its dots), and that grade. Editing comes in T16; the parts are laid out for it.
 */
export function NodeEditor({ grade, level, levels, onLevel, current, onCurrent, hint }: {
  grade: Grade | null;
  level: GradeLevel;
  levels: readonly GradeLevel[];
  onLevel: (level: GradeLevel) => void;
  current: string | null;
  onCurrent: (node: string) => void;
  /** What is said where there is no grade to show. */
  hint: string;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const said = words.hint.studio.colorPage;
  const complete = grade === null || isComplete(grade);
  return <div className="color-nodes-body">
    <div className="color-nodes-head">
      <h2 className="overline">{labels.nodeEditor}</h2>
      <span className="color-gap" />
      <div className="color-level-dots" role="group" aria-label={labels.level}>
        {levels.map(each => <button key={each} type="button" className="color-level-dot" aria-pressed={each === level} aria-label={labels.levels[each]}
          title={labels.levels[each]} onClick={() => onLevel(each)} data-level={each} />)}
      </div>
      <select className="field color-level" aria-label={labels.level} value={level} onChange={event => onLevel(event.target.value as GradeLevel)}
        data-test="node-level">
        {levels.map(each => <option key={each} value={each}>{labels.levels[each]}</option>)}
      </select>
    </div>
    {!complete && <p className="color-incomplete" role="status" data-test="grade-incomplete">{said.incomplete}</p>}
    <div className="color-nodes-canvas" data-test="node-canvas">
      {grade === null ? <p className="caption color-empty">{hint}</p> : <>
        <GradeGraph grade={grade} current={current} onCurrent={onCurrent} />
        {grade.nodes.length === 0 && <p className="caption color-empty" data-test="no-nodes">{said.noNodes}</p>}
      </>}
    </div>
  </div>;
}
