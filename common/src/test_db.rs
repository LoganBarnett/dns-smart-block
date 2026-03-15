use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, Once};
use std::time::Duration;

static INIT: Once = Once::new();
static PG_PROCESS: Mutex<Option<Child>> = Mutex::new(None);

/// PostgreSQL test database manager.
///
/// Automatically starts a PostgreSQL instance if needed, creates test
/// databases, and runs migrations.
pub struct TestDb {
  data_dir: PathBuf,
  port: u16,
}

impl TestDb {
  /// Initialize and start PostgreSQL if needed.
  ///
  /// This will:
  /// 1. Check if a PostgreSQL instance is already running
  /// 2. If not, initialize a new cluster with initdb
  /// 3. Start PostgreSQL on a free port
  /// 4. Create the test database
  /// 5. Run migrations
  pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .parent()
      .ok_or("Failed to find project root")?
      .to_path_buf();

    let data_dir = project_root.join("test-postgres-data");
    let port = 15432; // Use non-standard port to avoid conflicts

    INIT.call_once(|| {
      if let Err(e) = Self::ensure_postgres_running(&data_dir, port) {
        eprintln!("Failed to start PostgreSQL: {}", e);
        panic!("Failed to start test PostgreSQL instance");
      }
    });

    Ok(TestDb { data_dir, port })
  }

  /// Ensure PostgreSQL is running, starting it if necessary.
  fn ensure_postgres_running(
    data_dir: &PathBuf,
    port: u16,
  ) -> Result<(), Box<dyn std::error::Error>> {
    // Check if data directory exists and has been initialized
    let needs_init = !data_dir.join("PG_VERSION").exists();

    if needs_init {
      println!("Initializing PostgreSQL cluster in {:?}...", data_dir);
      std::fs::create_dir_all(data_dir)?;

      let output = Command::new("initdb")
        .arg("-D")
        .arg(data_dir)
        .arg("--no-locale")
        .arg("--encoding=UTF8")
        .output()?;

      if !output.status.success() {
        return Err(
          format!("initdb failed: {}", String::from_utf8_lossy(&output.stderr))
            .into(),
        );
      }
    }

    // Check if PostgreSQL is already running
    let pid_file = data_dir.join("postmaster.pid");
    if pid_file.exists() {
      println!("PostgreSQL already running (PID file exists)");
      return Ok(());
    }

    // Start PostgreSQL
    println!("Starting PostgreSQL on port {}...", port);
    let log_file = data_dir.join("postgres.log");
    let log = std::fs::File::create(&log_file)?;

    let mut child = Command::new("postgres")
      .arg("-D")
      .arg(data_dir)
      .arg("-p")
      .arg(port.to_string())
      .arg("-k")
      .arg(data_dir) // Unix socket directory
      .stdout(Stdio::from(log.try_clone()?))
      .stderr(Stdio::from(log))
      .spawn()?;

    // Wait for PostgreSQL to be ready
    println!("Waiting for PostgreSQL to be ready...");
    for i in 0..30 {
      std::thread::sleep(Duration::from_millis(100));

      let output = Command::new("pg_isready")
        .arg("-h")
        .arg(data_dir.to_str().unwrap())
        .arg("-p")
        .arg(port.to_string())
        .output();

      if let Ok(output) = output {
        if output.status.success() {
          println!("PostgreSQL ready after {}ms", i * 100);
          *PG_PROCESS.lock().unwrap() = Some(child);

          // Create test database
          Self::create_test_database(data_dir, port)?;
          return Ok(());
        }
      }
    }

    // If we get here, PostgreSQL didn't start in time
    let _ = child.kill();
    Err("PostgreSQL did not become ready in time".into())
  }

  /// Create the test database if it doesn't exist.
  fn create_test_database(
    data_dir: &PathBuf,
    port: u16,
  ) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating test database...");

    // Connect to default 'postgres' database to create our test database
    let output = Command::new("psql")
      .arg("-h")
      .arg(data_dir.to_str().unwrap())
      .arg("-p")
      .arg(port.to_string())
      .arg("-d")
      .arg("postgres")
      .arg("-c")
      .arg("CREATE DATABASE dns_smart_block_test;")
      .output();

    match output {
      Ok(output) => {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore error if database already exists
        if !output.status.success() && !stderr.contains("already exists") {
          return Err(
            format!("Failed to create test database: {}", stderr).into(),
          );
        }
        println!("Test database ready");
        Ok(())
      }
      Err(e) => Err(format!("Failed to run psql: {}", e).into()),
    }
  }

  /// Get the database URL for connecting to the test database.
  pub fn database_url(&self) -> String {
    format!(
      "postgresql://localhost:{}/dns_smart_block_test?host={}",
      self.port,
      self.data_dir.to_str().unwrap()
    )
  }

  /// Get a connection pool for the test database.
  pub async fn pool(&self) -> Result<sqlx::PgPool, sqlx::Error> {
    let pool = sqlx::PgPool::connect(&self.database_url()).await?;

    // Run migrations
    sqlx::migrate!("../migrations")
      .run(&pool)
      .await
      .expect("Failed to run migrations");

    Ok(pool)
  }
}

impl Drop for TestDb {
  fn drop(&mut self) {
    // PostgreSQL is intentionally left running so that multiple tests within
    // the same binary (which share the Once-initialized cluster) can all use
    // it without the first test to finish tearing it down for the rest.
    // The process is an OS-level child and will be cleaned up when the test
    // binary exits or when the developer runs `pg_ctl stop`.
  }
}
