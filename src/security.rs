pub fn redact(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "api_key",
                "api-key",
                "token",
                "secret",
                "password",
                "authorization: bearer",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                line.split_once('=')
                    .or_else(|| line.split_once(':'))
                    .map(|(key, _)| format!("{key}=***"))
                    .unwrap_or_else(|| line.to_owned())
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redacts_common_secret_formats() {
        let result = redact("API_KEY=secret\nAuthorization: Bearer abc\nnormal output");
        assert!(result.contains("API_KEY=***"));
        assert!(result.contains("Authorization=***"));
        assert!(result.contains("normal output"));
        assert!(!result.contains("secret"));
        assert!(!result.contains("abc"));
    }
}
