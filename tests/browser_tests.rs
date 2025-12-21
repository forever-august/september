//! Browser automation tests using thirtyfour
//!
//! These tests automatically start chromedriver and the application server.
//! Tests run in parallel by default since the server supports concurrent requests.
//!
//! Run with: cargo test --test browser_tests
use std::env;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use thirtyfour::prelude::*;

const SERVER_PORT: u16 = 3001;
const BASE_URL: &str = "http://127.0.0.1:3001";
const WEBDRIVER_PORT: u16 = 4444;
const WEBDRIVER_URL: &str = "http://localhost:4444";

/// Global chromedriver process manager
static CHROMEDRIVER: OnceLock<ChromeDriverManager> = OnceLock::new();

/// Global server process manager
static SERVER: OnceLock<ServerManager> = OnceLock::new();

/// Manages the chromedriver process lifecycle
struct ChromeDriverManager {
    process: Option<Child>,
}

impl ChromeDriverManager {
    /// Initialize the chromedriver manager, starting chromedriver if needed
    fn init() -> Self {
        if Self::is_running() {
            eprintln!("[test] chromedriver already running on port {}", WEBDRIVER_PORT);
            return Self { process: None };
        }

        eprintln!("[test] Starting chromedriver on port {}...", WEBDRIVER_PORT);

        let process = Command::new("chromedriver")
            .arg(format!("--port={}", WEBDRIVER_PORT))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start chromedriver. Is it installed?");

        let manager = Self {
            process: Some(process),
        };

        // Wait for chromedriver to be ready
        manager.wait_for_ready();

        manager
    }

    /// Check if chromedriver is already running
    fn is_running() -> bool {
        TcpStream::connect(format!("127.0.0.1:{}", WEBDRIVER_PORT)).is_ok()
    }

    /// Wait for chromedriver to be ready to accept connections
    fn wait_for_ready(&self) {
        let max_attempts = 50;
        let delay = Duration::from_millis(100);

        for attempt in 0..max_attempts {
            if Self::is_running() {
                eprintln!("[test] chromedriver ready after {} attempts", attempt + 1);
                return;
            }
            std::thread::sleep(delay);
        }

        panic!(
            "chromedriver did not start within {} seconds",
            (max_attempts as f64 * delay.as_secs_f64())
        );
    }
}

impl Drop for ChromeDriverManager {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.process {
            eprintln!("[test] Stopping chromedriver...");
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

/// Manages the application server process lifecycle
struct ServerManager {
    process: Option<Child>,
}

impl ServerManager {
    /// Initialize the server manager, building and starting the server if needed
    fn init() -> Self {
        if Self::is_running() {
            eprintln!("[test] Server already running on port {}", SERVER_PORT);
            return Self { process: None };
        }

        let project_root = Self::find_project_root();

        // Build the server
        eprintln!("[test] Building server...");
        let build_status = Command::new("cargo")
            .args(["build", "--bin", "september"])
            .current_dir(&project_root)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .expect("Failed to run cargo build");

        if !build_status.success() {
            panic!("Failed to build server");
        }

        let binary_path = project_root.join("target/debug/september");

        eprintln!("[test] Starting server on port {}...", SERVER_PORT);

        // Start the server with test configuration
        let config_path = project_root.join("config/test.toml");
        let process = Command::new(&binary_path)
            .current_dir(&project_root)
            .env("CONFIG_PATH", &config_path)
            .env("RUST_LOG", "september=warn")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit()) // Show server errors in test output
            .spawn()
            .expect("Failed to start server");

        let manager = Self {
            process: Some(process),
        };

        // Wait for server to be ready
        manager.wait_for_ready();

        manager
    }

    /// Find the project root directory
    fn find_project_root() -> PathBuf {
        // Try CARGO_MANIFEST_DIR first (set during cargo test)
        if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
            return PathBuf::from(manifest_dir);
        }

        // Fall back to current directory
        env::current_dir().expect("Failed to get current directory")
    }

    /// Check if the server is responding
    fn is_running() -> bool {
        TcpStream::connect(format!("127.0.0.1:{}", SERVER_PORT)).is_ok()
    }

    /// Wait for the server to be ready to accept connections
    fn wait_for_ready(&self) {
        let max_attempts = 100; // 10 seconds
        let delay = Duration::from_millis(100);

        // First wait for TCP port to be open
        for attempt in 0..max_attempts {
            if Self::is_running() {
                eprintln!("[test] Server TCP ready after {} attempts", attempt + 1);
                break;
            }
            std::thread::sleep(delay);
            if attempt == max_attempts - 1 {
                panic!("Server did not start within {} seconds",
                    (max_attempts as f64 * delay.as_secs_f64()));
            }
        }

        // Then do a warmup request to ensure NNTP workers are connected
        // This forces at least one worker to connect before tests begin
        eprintln!("[test] Warming up server (waiting for NNTP workers)...");
        for attempt in 0..max_attempts {
            match std::process::Command::new("curl")
                .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", BASE_URL])
                .output()
            {
                Ok(output) => {
                    let status = String::from_utf8_lossy(&output.stdout);
                    if status == "200" {
                        eprintln!("[test] Server fully ready after {} warmup attempts", attempt + 1);
                        return;
                    }
                }
                Err(_) => {}
            }
            std::thread::sleep(delay);
        }

        eprintln!("[test] Warning: Server warmup did not return 200, continuing anyway");
    }

    /// Assert that the server is still running (call before each test)
    fn assert_running() {
        if !Self::is_running() {
            panic!("Server crashed or is not responding on port {}", SERVER_PORT);
        }
    }
}

impl Drop for ServerManager {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.process {
            eprintln!("[test] Stopping server...");
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

/// Ensure chromedriver is running before tests
fn ensure_chromedriver() {
    CHROMEDRIVER.get_or_init(ChromeDriverManager::init);
}

/// Ensure the application server is running before tests
fn ensure_server() {
    SERVER.get_or_init(ServerManager::init);
    ServerManager::assert_running();
}

/// Ensure all test infrastructure is running
fn ensure_test_infrastructure() {
    ensure_chromedriver();
    ensure_server();
}

/// Helper to create a WebDriver instance (with visible browser)
#[allow(dead_code)]
async fn create_driver() -> WebDriverResult<WebDriver> {
    ensure_test_infrastructure();
    let caps = DesiredCapabilities::chrome();
    WebDriver::new(WEBDRIVER_URL, caps).await
}

/// Helper to create a headless WebDriver instance
async fn create_headless_driver() -> WebDriverResult<WebDriver> {
    ensure_test_infrastructure();
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--headless")?;
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--disable-gpu")?;
    WebDriver::new(WEBDRIVER_URL, caps).await
}

mod home_page {
    use super::*;

    #[tokio::test]
    async fn test_home_page_loads() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify the page title contains "September"
        let title = driver.title().await?;
        assert!(
            title.contains("September"),
            "Page title should contain 'September', got: {}",
            title
        );

        // Verify main container exists
        let main = driver.find(By::Tag("main")).await?;
        assert!(main.is_displayed().await?);

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_home_page_shows_page_header() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify page header is present
        let header = driver.find(By::ClassName("page-header")).await?;
        assert!(header.is_displayed().await?);

        // Verify the h1 contains "Newsgroups"
        let h1 = header.find(By::Tag("h1")).await?;
        let text = h1.text().await?;
        assert_eq!(text, "Newsgroups");

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_home_page_shows_group_tree() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify group tree container exists
        let tree_view = driver.find(By::Id("tree-view")).await?;
        assert!(tree_view.is_displayed().await?);

        // Verify there are tree items
        let tree_items = driver.find_all(By::ClassName("tree-item")).await?;
        assert!(!tree_items.is_empty(), "Tree should have items");

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_home_page_has_search_box() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify search input exists
        let search_input = driver.find(By::Id("group-search")).await?;
        assert!(search_input.is_displayed().await?);

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_group_links_are_clickable() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        // Find tree leaf links (actual groups)
        let leaf_links = driver.find_all(By::ClassName("tree-leaf")).await?;

        if !leaf_links.is_empty() {
            let first_link = &leaf_links[0];

            // Verify the link has an href starting with /g/
            let href = first_link.attr("href").await?;
            assert!(href.is_some(), "Group link should have href");
            assert!(
                href.as_ref().unwrap().starts_with("/g/"),
                "Group link should start with /g/"
            );
        }

        driver.quit().await?;
        Ok(())
    }
}

mod groups_page {
    use super::*;

    #[tokio::test]
    async fn test_groups_page_loads() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        // Verify the page title
        let title = driver.title().await?;
        assert!(
            title.contains("Groups"),
            "Page title should contain 'Groups', got: {}",
            title
        );

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_groups_page_has_header() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        // Verify page header
        let header = driver.find(By::ClassName("page-header")).await?;
        assert!(header.is_displayed().await?);

        let h1 = header.find(By::Tag("h1")).await?;
        let text = h1.text().await?;
        assert_eq!(text, "Newsgroups");

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_groups_page_has_group_list() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        // Wait for content to load
        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        // Verify group list container exists
        let group_list = driver.find(By::ClassName("group-list")).await?;
        assert!(group_list.is_displayed().await?);

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_group_cards_are_clickable() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        // Try to find group cards
        let group_cards = driver.find_all(By::ClassName("group-card")).await?;

        if !group_cards.is_empty() {
            let first_card = &group_cards[0];

            // Each group card should have a link
            let link = first_card.find(By::ClassName("group-link")).await?;
            let href = link.attr("href").await?;
            assert!(href.is_some(), "Group card should have a link with href");
            assert!(
                href.as_ref().unwrap().starts_with("/g/"),
                "Group link should start with /g/"
            );
        }

        driver.quit().await?;
        Ok(())
    }
}

mod navigation {
    use super::*;

    #[tokio::test]
    async fn test_navigate_from_home_to_groups_page() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Find and click the groups link in the header nav
        driver
            .set_implicit_wait_timeout(Duration::from_secs(5))
            .await?;

        // Look for a link to /groups in the navigation
        let groups_link = driver.find(By::Css(".site-nav a[href='/groups']")).await?;
        groups_link.click().await?;

        // Verify we're on the groups page
        let title = driver.title().await?;
        assert!(
            title.contains("Groups"),
            "Should navigate to groups page, got title: {}",
            title
        );

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_navigate_to_group_threads() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        // Find and click on a group
        let group_cards = driver.find_all(By::ClassName("group-card")).await?;

        if !group_cards.is_empty() {
            let first_card = &group_cards[0];
            let link = first_card.find(By::ClassName("group-link")).await?;
            link.click().await?;

            // Verify we're on a threads page
            let url = driver.current_url().await?;
            assert!(
                url.as_str().contains("/g/"),
                "URL should contain /g/, got: {}",
                url
            );
        }

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_click_thread_opens_thread_view() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        // Go to a group page to find threads (home page now shows group tree)
        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        // Click on the first group to go to its threads
        let group_cards = driver.find_all(By::ClassName("group-card")).await?;

        if !group_cards.is_empty() {
            let first_card = &group_cards[0];
            let link = first_card.find(By::ClassName("group-link")).await?;
            link.click().await?;

            // Wait for thread page to load
            tokio::time::sleep(Duration::from_secs(2)).await;

            // Find thread cards
            let thread_cards = driver.find_all(By::ClassName("thread-card")).await?;

            if !thread_cards.is_empty() {
                let first_card = &thread_cards[0];
                let title_link = first_card.find(By::Css(".thread-title a")).await?;
                title_link.click().await?;

                // Verify we're on a thread view page
                let url = driver.current_url().await?;
                assert!(
                    url.as_str().contains("/thread/"),
                    "URL should contain /thread/, got: {}",
                    url
                );
            }
        }

        driver.quit().await?;
        Ok(())
    }
}

mod thread_view {
    use super::*;

    #[tokio::test]
    async fn test_thread_view_displays_content() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        // Navigate to groups page and click into a group to find threads
        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        let group_cards = driver.find_all(By::ClassName("group-card")).await?;

        if !group_cards.is_empty() {
            // Click on the first group
            let first_card = &group_cards[0];
            let link = first_card.find(By::ClassName("group-link")).await?;
            link.click().await?;

            // Wait for thread list to load
            tokio::time::sleep(Duration::from_secs(2)).await;

            let thread_cards = driver.find_all(By::ClassName("thread-card")).await?;

            if !thread_cards.is_empty() {
                let first_card = &thread_cards[0];
                let title_link = first_card.find(By::Css(".thread-title a")).await?;
                title_link.click().await?;

                // Wait for thread view to load
                tokio::time::sleep(Duration::from_secs(2)).await;

                // Verify main content area exists
                let main = driver.find(By::Tag("main")).await?;
                assert!(main.is_displayed().await?);
            }
        }

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_thread_view_shows_all_replies() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        // Navigate to groups page and click into a group to find threads
        driver.goto(&format!("{}/groups", BASE_URL)).await?;

        driver
            .set_implicit_wait_timeout(Duration::from_secs(10))
            .await?;

        let group_cards = driver.find_all(By::ClassName("group-card")).await?;

        if group_cards.is_empty() {
            eprintln!("[test] No groups found, skipping reply verification");
            driver.quit().await?;
            return Ok(());
        }

        // Click on the first group
        let first_card = &group_cards[0];
        let link = first_card.find(By::ClassName("group-link")).await?;
        link.click().await?;

        // Wait for thread list to load
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Find all thread cards
        let thread_cards = driver.find_all(By::ClassName("thread-card")).await?;

        // Look for a thread that has replies (reply count > 0)
        let mut found_thread_with_replies = false;
        for card in &thread_cards {
            // Get the reply count from the thread card
            if let Ok(reply_count_elem) = card.find(By::ClassName("reply-count")).await {
                let reply_text = reply_count_elem.text().await?;
                // Parse the reply count (format: "X replies" or "1 reply")
                let reply_count: usize = reply_text
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                if reply_count > 0 {
                    found_thread_with_replies = true;
                    let expected_total = reply_count + 1; // replies + root message

                    // Click on this thread
                    let title_link = card.find(By::Css(".thread-title a")).await?;
                    title_link.click().await?;

                    // Wait for thread view to load
                    tokio::time::sleep(Duration::from_secs(2)).await;

                    // Verify thread stats shows correct article count
                    let thread_stats = driver.find(By::ClassName("thread-stats")).await?;
                    let stats_text = thread_stats.text().await?;
                    assert!(
                        stats_text.contains(&format!("{} messages", expected_total))
                            || stats_text.contains("1 message"),
                        "Thread stats should show message count, got: {}",
                        stats_text
                    );

                    // Verify that multiple comments are displayed in the thread tree
                    let comments = driver.find_all(By::ClassName("comment")).await?;
                    assert!(
                        comments.len() > 1,
                        "Thread with {} replies should display more than 1 comment, found {}",
                        reply_count,
                        comments.len()
                    );

                    break;
                }
            }
        }

        if !found_thread_with_replies {
            eprintln!("[test] No threads with replies found, skipping reply verification");
        }

        driver.quit().await?;
        Ok(())
    }
}

mod static_assets {
    use super::*;

    #[tokio::test]
    async fn test_stylesheet_loads() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify CSS link is present
        let css_link = driver
            .find(By::Css("link[href='/static/css/style.css']"))
            .await?;
        assert!(css_link.is_present().await?);

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_javascript_loads() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify JS script is present
        let js_script = driver
            .find(By::Css("script[src='/static/js/app.js']"))
            .await?;
        assert!(js_script.is_present().await?);

        driver.quit().await?;
        Ok(())
    }
}

mod responsive {
    use super::*;

    #[tokio::test]
    async fn test_viewport_meta_tag_present() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        driver.goto(BASE_URL).await?;

        // Verify viewport meta tag for responsive design
        let viewport = driver.find(By::Css("meta[name='viewport']")).await?;
        let content = viewport.attr("content").await?;
        assert!(
            content.is_some(),
            "Viewport meta tag should have content attribute"
        );

        driver.quit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_mobile_viewport() -> WebDriverResult<()> {
        let driver = create_headless_driver().await?;

        // Set mobile viewport size
        driver.set_window_rect(0, 0, 375, 667).await?;

        driver.goto(BASE_URL).await?;

        // Verify page still renders
        let main = driver.find(By::Tag("main")).await?;
        assert!(main.is_displayed().await?);

        driver.quit().await?;
        Ok(())
    }
}
