# Bootstrapping a project

Run `sdb init` in an existing directory, or give it a destination:

```sh
sdb init my-project
cd my-project
sdb validate --connect
sdb sql 'SELECT * FROM active_items ORDER BY id'
```

For development, invoke the command from this repository with
`cargo run -p semantic-cli -- init /path/to/my-project`.

The starter contains:

| Path | Purpose |
| --- | --- |
| `semantic-db.yaml` | Local CSV connection, source binding, and registered SQL view |
| `model.ossie.yaml` | Ossie semantic model with an `items` dataset and field descriptions |
| `data/items.csv` | Three synthetic items, usable without credentials |
| `views/active_items.sql` | Example view selecting the two active items |
| `.env.example` | Optional credentials and provider configuration for natural-language queries |
| `.gitignore` | Rules ignoring local `.env` files while allowing `.env.example` |

Replace the CSV and model with your own data and domain descriptions. Add SQL
files to `views/` and register each one under `views` in `semantic-db.yaml`.
The loader resolves model, source, and view paths relative to the configuration
file. From that directory the CLI discovers `semantic-db.yaml` automatically;
pass `--project-config PATH` to use another file or `--no-project` for an empty session.

To use natural-language queries, copy `.env.example` to `.env`, fill in
`OPENAI_API_KEY`, and run from the project directory so the CLI can find `.env`:

```sh
sdb ask 'List the active items ordered by id'
```

Initialization creates missing destination directories. It refuses existing
scaffold files or incompatible file/directory paths before writing the scaffold.
Existing unrelated files and `.env.example` are kept; the credential ignore rules
are appended to an existing `.gitignore`, preserving its contents. Re-running
`init` on an initialized project fails without replacing files. There is no force
option. Initialization cannot be combined with query or source flags.
