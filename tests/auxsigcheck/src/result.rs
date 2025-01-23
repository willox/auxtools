pub enum TestResult {
	Success,
	Failed(String),
	Missing(String)
}

pub fn extract_test_result(s: &str) -> TestResult {
	for line in s.lines() {
		let trimmed = line.trim();
		if trimmed.is_empty() {
			continue;
		} else if trimmed.starts_with("SUCCESS") {
			return TestResult::Success;
		} else if trimmed.starts_with("FAILED") {
			return TestResult::Failed(trimmed.split('(').nth(1).unwrap().replace(")", "").trim().to_owned());
		} else if trimmed.starts_with("MISSING") {
			return TestResult::Missing(trimmed.split(':').nth(1).unwrap().trim().to_owned());
		}
	}
	panic!("No test result found")
}
