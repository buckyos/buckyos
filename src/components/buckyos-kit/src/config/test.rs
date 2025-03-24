use super::merge::ConfigMerger;
use std::io::Write;
use std::path::Path;

const RESULT: &str = r#"
{
  "app": {
    "name": "ExampleApp",
    "version": "1.1.0",
    "description": "Subdir1 App"
  },
  "logging": {
    "level": "warn"
  },
  "workers": {
    "count": 8
  },
  "database": {
    "host": "localhost",
    "port": 5432,
    "username": "admin",
    "password": "password123",
    "ssl": true
  }
}
"#;

fn create_test_directory_without_root(base_dir: &Path) -> std::io::Result<()> {
    // main directory
    let main_files = vec![
        (
            "file.1.json",
            r#"
        {
          "app": {
            "name": "ExampleApp",
            "version": "1.0.0"
          },
          "logging": {
            "level": "info"
          }
        }
        "#,
        ),
        (
            "file.2.toml",
            r#"
        [app]
        version = "1.1.0" # Replace version in JSON file 1

        [logging]
        level = "debug" #  Update log level

        [database]
        host = "localhost"
        port = 5432
        "#,
        ),
    ];

    // sub dir 1
    let subdir1_files = vec![
        (
            "subfile.1.json",
            r#"
        {
          "app": {
            "description": "Subdir1 App"
          },
          "workers": {
            "count": 4
          }
        }
        "#,
        ),
        (
            "subfile.3.toml",
            r#"
        [workers]
        count = 8 # Update workers count

        [logging]
        level = "warn" # Replace log level
        "#,
        ),
    ];

    // sub dir 2
    let subdir2_files = vec![
        (
            "subfile.2.json",
            r#"
        {
          "database": {
            "username": "admin",
            "password": "password123",
            "ssl": false
          }
        }
        "#,
        ),
        (
            "subfile.4.toml",
            r#"
        [database]
        ssl = true # Update SSL setting
        "#,
        ),
    ];

    // Create main directory and files
    std::fs::create_dir_all(base_dir)?;
    for (name, content) in &main_files {
        let file_path = base_dir.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    // Create sub directory 1 and files
    let subdir1 = base_dir.join("subdir.3");
    std::fs::create_dir_all(&subdir1)?;
    for (name, content) in &subdir1_files {
        let file_path = subdir1.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    // Create sub directory 2 and files
    let subdir2 = base_dir.join("subdir.4");
    std::fs::create_dir_all(&subdir2)?;
    for (name, content) in &subdir2_files {
        let file_path = subdir2.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    Ok(())
}

fn create_test_directory_with_root(base_dir: &Path) -> std::io::Result<()> {
    // main directory
    let main_files = vec![
        (
            "root",
            r#"
          [[includes]]
          path = "file1.json"

          [[includes]]
          path = "file2.toml"

          [[includes]]
          path = "subdir3"

          [[includes]]
          path = "subdir4"
          "#,
        ),
        (
            "file1.json",
            r#"
      {
        "app": {
          "name": "ExampleApp",
          "version": "1.0.0"
        },
        "logging": {
          "level": "info"
        }
      }
      "#,
        ),
        (
            "file2.toml",
            r#"
      [app]
      version = "1.1.0" # Replace version in JSON file 1

      [logging]
      level = "debug" #  Update log level

      [database]
      host = "localhost"
      port = 5432
      "#,
        ),
    ];

    // sub dir 1
    let subdir1_files = vec![
        (
            "root.json",
            r#"
        {
          "includes": [
            { "path": "subfile1.json" },
            { "path": "subfile3.toml" }
          ]
        }
        "#,
        ),
        (
            "subfile1.json",
            r#"
      {
        "app": {
          "description": "Subdir1 App"
        },
        "workers": {
          "count": 4
        }
      }
      "#,
        ),
        (
            "subfile3.toml",
            r#"
      [workers]
      count = 8 # Update workers count

      [logging]
      level = "warn" # Replace log level
      "#,
        ),
    ];

    // sub dir 2
    let subdir2_files = vec![
        (
            "root.toml",
            r#"
        [[includes]]
        path = "subfile2.json"

        [[includes]]
        path = "subfile4.toml"
        "#,
        ),
        (
            "subfile2.json",
            r#"
      {
        "database": {
          "username": "admin",
          "password": "password123",
          "ssl": false
        }
      }
      "#,
        ),
        (
            "subfile4.toml",
            r#"
      [database]
      ssl = true # Update SSL setting
      "#,
        ),
    ];

    // Create main directory and files
    std::fs::create_dir_all(base_dir)?;
    for (name, content) in &main_files {
        let file_path = base_dir.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    // Create sub directory 1 and files
    let subdir1 = base_dir.join("subdir3");
    std::fs::create_dir_all(&subdir1)?;
    for (name, content) in &subdir1_files {
        let file_path = subdir1.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    // Create sub directory 2 and files
    let subdir2 = base_dir.join("subdir4");
    std::fs::create_dir_all(&subdir2)?;
    for (name, content) in &subdir2_files {
        let file_path = subdir2.join(name);
        let mut file = std::fs::File::create(file_path)?;
        file.write_all(content.trim_start().as_bytes())?;
    }

    Ok(())
}

#[tokio::test]
async fn main() {
    std::env::set_var("BUCKY_LOG", "debug");
    crate::log_util::init_logging("test_config",false);

    // Get temp directory on current system
    let temp_dir = std::env::temp_dir();

    // Test without root file
    {
        let base_dir = temp_dir.join("buckyos/config");
        std::fs::create_dir_all(&base_dir).unwrap();

        println!("Base dir: {:?}", base_dir);

        create_test_directory_without_root(&base_dir).unwrap();

        let value = ConfigMerger::load_dir(&base_dir).await.unwrap();
        let s = serde_json::to_string_pretty(&value).unwrap();
        println!("Merged config: {:?}", s);

        // Compare the merged config with expected result
        let expected: serde_json::Value = serde_json::from_str(RESULT).unwrap();
        assert_eq!(value, expected);
    }

    // Test with root file
    {
        let base_dir = temp_dir.join("buckyos/config2");
        std::fs::create_dir_all(&base_dir).unwrap();

        println!("Base dir: {:?}", base_dir);

        create_test_directory_with_root(&base_dir).unwrap();

        let value = ConfigMerger::load_dir(&base_dir).await.unwrap();
        let s = serde_json::to_string_pretty(&value).unwrap();
        println!("Merged config: {:?}", s);

        // Compare the merged config with expected result
        let expected: serde_json::Value = serde_json::from_str(RESULT).unwrap();
        assert_eq!(value, expected);
    }
}
