# Hello KPI

A counter card. Bind `tasks` to a table that has a numeric `hours` field and a select field `status`.
Config: `label` (text, shown above the number), `accent` (color). Insert it with a `buckyos.cell`
whose `view.type` is `declarative`, `def_ref` names this definition and `bindings.tasks.entity_id` the table.
