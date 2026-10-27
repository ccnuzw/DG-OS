#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_serialization() {
        use serde_json;
        
        let cap = Capability::TextGeneration;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"text-generation\"");
        
        let cap: Capability = serde_json::from_str("\"image-generation\"").unwrap();
        assert_eq!(cap, Capability::ImageGeneration);
    }
}
