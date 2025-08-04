use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Creates a temporary test project with Rust code for testing
pub fn create_test_project(temp_dir: &TempDir) -> std::io::Result<()> {
    let project_dir = temp_dir.path().join("test_project");
    fs::create_dir_all(&project_dir)?;

    // Create src directory
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    // Create a simple Rust file with various code structures
    let main_rs = src_dir.join("main.rs");
    let main_content = r#"
use std::collections::HashMap;

/// A simple struct for testing
#[derive(Debug, Clone)]
pub struct TestStruct {
    pub name: String,
    pub value: i32,
}

impl TestStruct {
    /// Creates a new TestStruct
    pub fn new(name: String, value: i32) -> Self {
        Self { name, value }
    }
    
    /// Gets the name of the struct
    pub fn get_name(&self) -> &str {
        &self.name
    }
    
    /// Gets the value of the struct
    pub fn get_value(&self) -> i32 {
        self.value
    }
}

/// A function that processes data
pub fn process_data(data: &[i32]) -> HashMap<String, i32> {
    let mut result = HashMap::new();
    for (i, &value) in data.iter().enumerate() {
        result.insert(format!("item_{}", i), value);
    }
    result
}

fn main() {
    let test_struct = TestStruct::new("test".to_string(), 42);
    println!("Name: {}", test_struct.get_name());
    println!("Value: {}", test_struct.get_value());
    
    let data = vec![1, 2, 3, 4, 5];
    let processed = process_data(&data);
    println!("Processed data: {:?}", processed);
}
"#;
    fs::write(main_rs, main_content)?;

    // Create a second file for more complex testing
    let utils_rs = src_dir.join("utils.rs");
    let utils_content = r#"
/// Utility functions for the test project
pub mod utils {
    use std::collections::HashMap;
    
    /// A trait for data processing
    pub trait DataProcessor {
        fn process(&self, data: &[i32]) -> HashMap<String, i32>;
    }
    
    /// A concrete implementation of DataProcessor
    pub struct SimpleProcessor;
    
    impl DataProcessor for SimpleProcessor {
        fn process(&self, data: &[i32]) -> HashMap<String, i32> {
            let mut result = HashMap::new();
            for (i, &value) in data.iter().enumerate() {
                result.insert(format!("processed_{}", i), value * 2);
            }
            result
        }
    }
    
    /// A more complex struct with generics
    pub struct ComplexStruct<T> {
        pub data: Vec<T>,
        pub metadata: HashMap<String, String>,
    }
    
    impl<T> ComplexStruct<T> {
        pub fn new(data: Vec<T>) -> Self {
            Self {
                data,
                metadata: HashMap::new(),
            }
        }
        
        pub fn add_metadata(&mut self, key: String, value: String) {
            self.metadata.insert(key, value);
        }
        
        pub fn get_metadata(&self, key: &str) -> Option<&String> {
            self.metadata.get(key)
        }
    }
}
"#;
    fs::write(utils_rs, utils_content)?;

    // Create Cargo.toml
    let cargo_toml = project_dir.join("Cargo.toml");
    let cargo_content = r#"
[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;
    fs::write(cargo_toml, cargo_content)?;

    Ok(())
}

/// Creates a test project with multiple files and complex structures
pub fn create_complex_test_project(temp_dir: &TempDir) -> std::io::Result<()> {
    let project_dir = temp_dir.path().join("complex_test_project");
    fs::create_dir_all(&project_dir)?;

    // Create src directory
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    // Create lib.rs
    let lib_rs = src_dir.join("lib.rs");
    let lib_content = r#"
pub mod models;
pub mod services;
pub mod utils;

/// Main library module
pub struct Library {
    pub name: String,
    pub version: String,
}

impl Library {
    pub fn new(name: String, version: String) -> Self {
        Self { name, version }
    }
    
    pub fn get_info(&self) -> String {
        format!("{} v{}", self.name, self.version)
    }
}
"#;
    fs::write(lib_rs, lib_content)?;

    // Create models.rs
    let models_rs = src_dir.join("models.rs");
    let models_content = r#"
use serde::{Deserialize, Serialize};

/// User model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub active: bool,
}

impl User {
    pub fn new(id: u64, name: String, email: String) -> Self {
        Self {
            id,
            name,
            email,
            active: true,
        }
    }
    
    pub fn deactivate(&mut self) {
        self.active = false;
    }
    
    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// Product model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub price: f64,
    pub category: String,
}

impl Product {
    pub fn new(id: String, name: String, price: f64, category: String) -> Self {
        Self {
            id,
            name,
            price,
            category,
        }
    }
    
    pub fn get_display_price(&self) -> String {
        format!("${:.2}", self.price)
    }
}
"#;
    fs::write(models_rs, models_content)?;

    // Create services.rs
    let services_rs = src_dir.join("services.rs");
    let services_content = r#"
use crate::models::{User, Product};
use std::collections::HashMap;

/// User service for managing users
pub struct UserService {
    users: HashMap<u64, User>,
}

impl UserService {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }
    
    pub fn add_user(&mut self, user: User) {
        self.users.insert(user.id, user);
    }
    
    pub fn get_user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }
    
    pub fn list_users(&self) -> Vec<&User> {
        self.users.values().collect()
    }
}

/// Product service for managing products
pub struct ProductService {
    products: HashMap<String, Product>,
}

impl ProductService {
    pub fn new() -> Self {
        Self {
            products: HashMap::new(),
        }
    }
    
    pub fn add_product(&mut self, product: Product) {
        self.products.insert(product.id.clone(), product);
    }
    
    pub fn get_product(&self, id: &str) -> Option<&Product> {
        self.products.get(id)
    }
    
    pub fn list_products(&self) -> Vec<&Product> {
        self.products.values().collect()
    }
}
"#;
    fs::write(services_rs, services_content)?;

    // Create utils.rs
    let utils_rs = src_dir.join("utils.rs");
    let utils_content = r#"
use std::collections::HashMap;

/// Utility functions for the library
pub mod utils {
    /// Validates an email address
    pub fn validate_email(email: &str) -> bool {
        email.contains('@') && email.contains('.')
    }
    
    /// Formats a price with currency
    pub fn format_price(price: f64, currency: &str) -> String {
        format!("{}{:.2}", currency, price)
    }
    
    /// Generates a unique ID
    pub fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        format!("id_{}", timestamp)
    }
}
"#;
    fs::write(utils_rs, utils_content)?;

    // Create Cargo.toml
    let cargo_toml = project_dir.join("Cargo.toml");
    let cargo_content = r#"
[package]
name = "complex_test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
"#;
    fs::write(cargo_toml, cargo_content)?;

    Ok(())
}

/// Helper function to check if Qdrant is running
pub async fn is_qdrant_running() -> bool {
    match reqwest::get("http://localhost:6334/collections").await {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Helper function to wait for Qdrant to be ready
pub async fn wait_for_qdrant(max_attempts: u32) -> bool {
    for attempt in 1..=max_attempts {
        if is_qdrant_running().await {
            return true;
        }

        if attempt < max_attempts {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
    false
}
