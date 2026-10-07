use serde_json::json;
use speran::error::ErrorKind;
use speran::interp::Limits;
use speran::tools::ToolRegistry;
use speran::value::Value;
use speran::SperanRunner;

#[test]
fn test_basic_eval() {
    let mut runner = SperanRunner::new();
    let res = runner.execute("x = 40; y = 2; return x + y");
    assert!(res.ok);
    assert_eq!(res.value, Value::Num(42.0));
}

#[test]
fn test_comprehension_and_interpolation() {
    let mut runner = SperanRunner::new();
    let code = r#"
        nums = [1, 2, 3, 4]
        evens = [n * 2 for n in nums if n % 2 == 0]
        greeting = "Evens: {join(evens, \", \")}"
        return {evens: evens, msg: greeting}
    "#;
    let res = runner.execute(code);
    assert!(res.ok, "Execution failed: {:?}", res.error);
    if let Value::Map(m) = res.value {
        assert_eq!(m.get("evens"), Some(&Value::list(vec![Value::Num(4.0), Value::Num(8.0)])));
        assert_eq!(m.get("msg"), Some(&Value::str("Evens: 4, 8")));
    } else {
        panic!("Expected map value");
    }
}

#[test]
fn test_recursive_function_and_scoping() {
    let mut runner = SperanRunner::new();
    let code = r#"
        fn fact(n) {
            if n <= 1 {
                return 1
            }
            return n * fact(n - 1)
        }
        return fact(5)
    "#;
    let res = runner.execute(code);
    assert!(res.ok);
    assert_eq!(res.value, Value::Num(120.0));
}

#[test]
fn test_method_call_llm_hint() {
    let mut runner = SperanRunner::new();
    let code = r#"
        items = [1, 2, 3]
        items.append(4)
    "#;
    let res = runner.execute(code);
    assert!(!res.ok);
    let err = res.error.expect("Expected static error for method call");
    assert_eq!(err.kind, ErrorKind::SyntaxError);
    assert!(err.hint.unwrap().contains("items = items + [item]"));
}

#[test]
fn test_string_method_llm_hint() {
    let mut runner = SperanRunner::new();
    let code = r#"
        name = "alice"
        return name.upper()
    "#;
    let res = runner.execute(code);
    assert!(!res.ok);
    let err = res.error.expect("Expected static error for upper() method");
    assert_eq!(err.kind, ErrorKind::SyntaxError);
    assert!(err.hint.unwrap().contains("upper(name)"));
}

#[test]
fn test_did_you_mean_variable_typo() {
    let mut runner = SperanRunner::new();
    let code = r#"
        counter = 10
        return countr + 1
    "#;
    let res = runner.execute(code);
    assert!(!res.ok);
    let err = res.error.expect("Expected UnknownName error");
    assert_eq!(err.kind, ErrorKind::UnknownName);
    assert_eq!(err.hint, Some("Did you mean 'counter'?".to_string()));
}

#[test]
fn test_step_limit_prevents_infinite_loop() {
    let mut runner = SperanRunner::new().with_limits(Limits {
        max_steps: 100,
        max_tool_calls: 10,
        timeout_ms: None,
        max_call_depth: 32,
    });
    let code = r#"
        while true {
            x = 1
        }
    "#;
    let res = runner.execute(code);
    assert!(!res.ok);
    let err = res.error.expect("Expected StepLimitExceeded");
    assert_eq!(err.kind, ErrorKind::StepLimitExceeded);
}

#[test]
fn test_tool_capabilities_and_tracing() {
    let mock_json = json!({
        "calc.multiply": {
            "$match": "x",
            "cases": {
                "5": 25,
                "10": 100
            },
            "default": 0
        }
    });

    let tools = ToolRegistry::from_mock_json(&mock_json).unwrap();
    let mut runner = SperanRunner::new()
        .with_tools(tools)
        .with_grants(vec!["calc.multiply".to_string()]);

    let code = r#"
        use calc.multiply
        a = calc.multiply(x: 5)
        b = calc.multiply(x: 10)
        return a + b
    "#;
    let res = runner.execute(code);
    assert!(res.ok, "Execution failed: {:?}", res.error);
    assert_eq!(res.value, Value::Num(125.0));
    assert_eq!(res.trace.len(), 2);
    assert_eq!(res.trace[0].tool, "calc.multiply");
    assert_eq!(res.tool_calls_used, 2);
}

#[test]
fn test_unauthorized_tool_denied() {
    let mock_json = json!({
        "fs.delete": {"deleted": true}
    });
    let tools = ToolRegistry::from_mock_json(&mock_json).unwrap();
    // Host only grants read tools
    let mut runner = SperanRunner::new()
        .with_tools(tools)
        .with_grants(vec!["fs.read".to_string()]);

    let code = r#"
        use fs.delete
        return fs.delete(path: "/important")
    "#;
    let res = runner.execute(code);
    assert!(!res.ok);
    let err = res.error.expect("Expected CapabilityDenied");
    assert_eq!(err.kind, ErrorKind::CapabilityDenied);
}

#[test]
fn test_try_expression_catches_non_fatal_errors() {
    let mock_json = json!({
        "api.fetch": {
            "$error": "Connection refused"
        }
    });
    let tools = ToolRegistry::from_mock_json(&mock_json).unwrap();
    let mut runner = SperanRunner::new().with_tools(tools);

    let code = r#"
        use api.fetch
        res = try api.fetch(endpoint: "/bad")
        if res.error {
            return {caught: true, msg: res.error.message}
        }
        return {caught: false}
    "#;
    let report = runner.execute(code);
    assert!(report.ok);
    if let Value::Map(m) = report.value {
        assert_eq!(m.get("caught"), Some(&Value::Bool(true)));
    } else {
        panic!("Expected map");
    }
}
