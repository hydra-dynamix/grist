# SQLite inspection

Feature: `sqlite` (included by `structured-data` and `full`)

Grist reads SQLite format-3 bytes directly. It does not link to or invoke a SQL
engine, open a write transaction, replay journals, evaluate view or trigger SQL,
load extensions, register functions, or accept arbitrary SQL. Schema SQL is
retained only as inert source text.

## Output and selection

`grist/sqlite/v1` preserves the database header, every visible
`sqlite_schema` object, table column declarations, view definitions, index
definitions, and inert trigger definitions. Internal `sqlite_*` objects are
excluded by default and can be inventoried with `include_internal_schema`.

Record extraction is off by default. To extract records, callers must supply
`record_selection` with:

- one or more exact table names;
- a positive `max_tables`; and
- a positive `max_rows_per_table`.

```json
{
  "record_selection": {
    "tables": ["users"],
    "max_tables": 1,
    "max_rows_per_table": 100
  },
  "include_internal_schema": false
}
```

Rows carry a stable native key (`rowid` for ordinary tables, a canonical value
hash for `WITHOUT ROWID` storage), physical page/cell evidence, exact source
byte bounds, and nested record/field locators. Table, row, and field nodes are
projected into `DocumentGraph`.

Malformed headers, invalid pages, cyclic B-trees, broken overflow chains,
missing selected tables, invalid selection options, cancellation, and resource
limits produce specific diagnostics. A row/table limit is `partial`, never an
implicit end of data.

`inspect_sqlite_path` opens only the main file with read permission. If the
file cannot be opened (including an exclusive lock), it returns
`sqlite.io.locked_or_unreadable`. Non-empty `-wal` or `-journal` sidecars
are reported as `sqlite.sidecar.not_replayed` and make the result partial;
Grist never performs recovery because that can mutate the source.

`WITHOUT ROWID` records are exposed in physical key order and produce an
explicit partial diagnostic. Virtual tables and rootless objects remain in
schema output; selecting records from them produces
`sqlite.table.no_storage`.
