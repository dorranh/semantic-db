use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

const FILES: &[(&str, &str)] = &[
    (
        "semantic-db.yaml",
        include_str!("../templates/semantic-db.yaml"),
    ),
    (
        "model.ossie.yaml",
        include_str!("../templates/model.ossie.yaml"),
    ),
    ("data/items.csv", include_str!("../templates/items.csv")),
    (
        "views/active_items.sql",
        include_str!("../templates/active_items.sql"),
    ),
];
const ENV: &str = include_str!("../templates/env.example");
const IGNORE: &str = "# Semantic DB local credentials\n/.env\n/.env.*\n!/.env.example\n";

pub fn run(root: &Path) -> super::Result<()> {
    // Check all known conflicts before writing anything. create_new also protects
    // existing files if another process creates a target after this check.
    for directory in [root.to_path_buf(), root.join("data"), root.join("views")] {
        if let Some(metadata) = metadata(&directory)?
            && !metadata.is_dir()
        {
            return Err(format!("expected a project directory: {}", directory.display()).into());
        }
    }
    for (name, _) in FILES {
        let path = root.join(name);
        if metadata(&path)?.is_some() {
            return Err(format!(
                "refusing to overwrite {}; choose another project directory",
                path.display()
            )
            .into());
        }
    }
    for name in [".env.example", ".gitignore"] {
        let path = root.join(name);
        if let Some(metadata) = metadata(&path)?
            && !metadata.is_file()
        {
            return Err(format!("expected a regular file: {}", path.display()).into());
        }
    }
    let ignore_path = root.join(".gitignore");
    let existing_ignore = match fs::read(&ignore_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    fs::create_dir_all(root.join("data"))?;
    fs::create_dir_all(root.join("views"))?;
    for (name, contents) in FILES {
        write_new(&root.join(name), contents)?;
    }
    if metadata(&root.join(".env.example"))?.is_none() {
        write_new(&root.join(".env.example"), ENV)?;
    }
    if let Some(bytes) = existing_ignore {
        // Append the project rules so existing ignore patterns and comments stay intact.
        let mut file = OpenOptions::new().append(true).open(&ignore_path)?;
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            file.write_all(b"\n")?;
        }
        file.write_all(IGNORE.as_bytes())?;
    } else {
        write_new(&ignore_path, IGNORE)?;
    }

    println!("Initialized Semantic DB project in {}", root.display());
    println!(
        "Created semantic-db.yaml, model.ossie.yaml, data/items.csv, and views/active_items.sql."
    );
    println!("Prepared .env.example and .gitignore (existing environment templates are kept).");
    println!("\nFrom the project directory, run:");
    println!("  semantic-db --config semantic-db.yaml --validate --connect");
    println!(
        "  semantic-db --config semantic-db.yaml --query 'SELECT * FROM active_items ORDER BY id'"
    );
    println!("  semantic-db --config semantic-db.yaml");
    println!("\nEdit the model, source bindings, and SQL views to use your own data.");
    println!("For Ask, copy .env.example to .env and set OPENAI_API_KEY and OPENAI_MODEL.");
    println!("  semantic-db --config semantic-db.yaml --ask-views 'List active items' --dry-run");
    println!("  semantic-db --config semantic-db.yaml --ask-views 'List active items'");
    println!("For applications, start the same project with:");
    println!("  semantic-server --config semantic-db.yaml");
    Ok(())
}

fn metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_new(path: &Path, contents: &str) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(contents.as_bytes())
}
