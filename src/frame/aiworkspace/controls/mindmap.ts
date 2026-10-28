/* The `mindmap` control (脑图方案 §6.2, R2): the first built-in Block-control. What every reader shares — its
 * catalogue entry and the self-description an Agent reads (R11, §8.1) — lives here; the Desktop's ContainerDefinition
 * adds the canvas behaviour. */

import type { ControlDescriptor } from './catalog.ts'

const AGENT = `# mindmap — a mind map control

A \`buckyos.container\` with \`kind: "mindmap"\` on a canvas (a Surface or a group). Its nodes are ordinary Blocks
(\`buckyos.cell\`): \`parent_id\` is the tree (a node's children sit under it), \`order_key\` the sibling order. The
control lays the nodes out and draws the edges between parent and child; nothing about edges is stored.

## Rules

- **Do not compute coordinates.** Give every node \`placement: { "x": "auto", "y": "auto", "w": "auto", "h": "auto" }\`
  (or no placement at all). The control's \`tree\` layout places them; a number in a component pins it.
- The control itself: \`placement: { x, y, w: "auto", h: "auto" }\` with numbers for x / y (its first root's
  top-left on the canvas; omit placement on a flow page). Its \`layout\` is \`{ "mode": "tree" }\` (implied).
- Text nodes: \`view: { type: "mindnode" }\`, the text in \`title\` (at most 256 characters; inline Markdown —
  **bold**, *italic*, \`code\`, [links](https://…) — is kept and shown formatted).
- A node may be any Block (an image, a table view, a shape…). A node that is not a text view (mindnode, shape,
  richtext) needs numeric \`w\` / \`h\`: \`{ "x": "auto", "y": "auto", "w": 240, "h": 160 }\`.
- A node may itself be another mindmap control (its own tree inside one node).
- Normally one root node directly under the container; several roots are stacked.
- Relation lines between nodes are ordinary connectors on the canvas (\`view.type: connector\`), not children.
- Folding is each reader's own view state: you always see (and write) the whole tree.

## Config (\`payload.config\`, all optional)

| key | values | default |
| --- | --- | --- |
| \`direction\` | \`right\` · \`left\` · \`both\` (root in the middle) · \`down\` (org chart) | \`right\` |
| \`edge\` | \`curve\` · \`elbow\` · \`straight\` | \`curve\` |
| \`spacing\` | \`{ sibling, level }\` in px (0–2000) | \`{ sibling: 12, level: 48 }\` |
| \`branch_colors\` | up to 64 \`#rrggbb\` colours for the first-level branches | a built-in palette |

A node's \`config\` may carry \`branch_color\` (\`#rrggbb\`, inherited by its subtree) and, on a first-level node of a
\`both\` map, \`side: "left" | "right"\`.

## Create one with doc.commit

\`\`\`json
[
  { "op": "entity.create", "entity_id": "mm1", "type_id": "buckyos.container", "parent_id": "<surface id>", "order_key": "m",
    "placement": { "x": 100, "y": 100, "w": "auto", "h": "auto" },
    "payload": { "kind": "mindmap", "title": "Launch plan", "config": { "direction": "both" } } },
  { "op": "entity.create", "entity_id": "mm1-root", "type_id": "buckyos.cell", "parent_id": "mm1", "order_key": "a",
    "placement": { "x": "auto", "y": "auto", "w": "auto", "h": "auto" },
    "payload": { "view": { "type": "mindnode" }, "title": "Launch plan" } },
  { "op": "entity.create", "entity_id": "mm1-a", "type_id": "buckyos.cell", "parent_id": "mm1-root", "order_key": "a",
    "placement": { "x": "auto", "y": "auto", "w": "auto", "h": "auto" },
    "payload": { "view": { "type": "mindnode" }, "title": "Market" } },
  { "op": "entity.create", "entity_id": "mm1-a1", "type_id": "buckyos.cell", "parent_id": "mm1-a", "order_key": "a",
    "placement": { "x": "auto", "y": "auto", "w": "auto", "h": "auto" },
    "payload": { "view": { "type": "mindnode" }, "title": "**User** interviews" } }
]
\`\`\`

Add a branch later: \`entity.create\` Cells under an existing node (\`parent_id\` = that node, an \`order_key\` after its
last child). Move a branch: \`tree.move\` the node to another node, placement stays \`"auto"\`. Delete a branch:
\`entity.delete\` the node with \`subtree: { delete: [every descendant id] }\`.

## Exchange formats

- **Markdown outline** (preferred): a nested list, optionally under headings; one top-level item is the root.
  \`buckyos aiws import <file.md> --workspace <ws> --parent <surface id> [--at x,y]\` creates the map;
  \`buckyos aiws export <control id> --workspace <ws> [--format opml]\` reads it back. In a wish:
  \`aiws.result.control(name, { kind: "mindmap", children: "<markdown outline>" })\`; to grow a node that is an
  input, add \`under: "<input name>"\` (its current branch reads as an outline).
- **OPML**: the same tree as \`<outline text>\` elements (\`--format opml\`, or an \`.opml\` file to import).

## Don'ts

- No numbers for node x / y / w / h unless you mean to pin a node (and a non-text node's size).
- No connectors inside the control, no data entities under it, no \`layout\` other than \`{ mode: tree }\`.
- Nodes stay inside their control: to put a branch elsewhere, create a copy there and delete the original.
`

export const MINDMAP: ControlDescriptor = {
  kind: 'mindmap',
  title: 'Mind map',
  summary: 'A tree of ideas laid out automatically from a root, with edges drawn by the control.',
  when: 'Brainstorming, outlines, plans, taxonomies, org charts (direction down) — any hierarchy a reader takes in at a glance.',
  formats: ['markdown', 'opml'],
  agent: AGENT,
}
