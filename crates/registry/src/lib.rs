//! The only crate that names concrete drivers. Each driver sits behind a Cargo feature of the
//! same name; `fake` adds the in-memory test drivers.

use traits::DriverFactory;

/// Every compiled-in driver, in picker order.
#[must_use]
pub fn all() -> Vec<Box<dyn DriverFactory>> {
    vec![
        #[cfg(feature = "postgres")]
        postgres_driver::factory(),
        #[cfg(feature = "mysql")]
        mysql_driver::factory(),
        #[cfg(feature = "sqlite")]
        sqlite_driver::factory(),
        #[cfg(feature = "fake")]
        Box::new(testkit::FakeSqlFactory::default()),
        #[cfg(feature = "fake")]
        Box::new(testkit::FakeKvFactory::default()),
    ]
}

#[cfg(all(test, feature = "fake"))]
mod tests {
    #[test]
    fn fake_feature_registers_fakes() {
        let ids: Vec<String> = super::all()
            .iter()
            .map(|f| f.info().id.to_string())
            .collect();
        assert!(ids.ends_with(&["fake-sql".to_owned(), "fake-kv".to_owned()]));
    }
}
