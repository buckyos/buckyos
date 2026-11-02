# Hello Board

A panel that stacks task cards (a composite control). An instance is a `buckyos.container { kind: "composite",
def_ref: { entity_id: <this definition> }, config: { title?, background?, gap? } }`; put `hello-card` Cells (or rich
text notes) under it with `slot: "cards"`, and at most one header note with `slot: "header"`. The board lays them out
in a column (`gap` px apart) and draws the frame; the cards stay ordinary Blocks. Copy an instance with `tree.clone`.
