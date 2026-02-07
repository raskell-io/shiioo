//! Built-in skills that ship with Shiioo.
//!
//! These skills provide common capabilities for agents without requiring
//! external skill sources.

use super::{Skill, SkillCategory, SkillMetadata};

/// Get all builtin skill metadata for discovery.
pub fn all_metadata() -> Vec<SkillMetadata> {
    vec![
        code_review_metadata(),
        debugging_metadata(),
        testing_metadata(),
        architecture_review_metadata(),
        incident_response_metadata(),
        data_analysis_metadata(),
        sql_queries_metadata(),
        security_review_metadata(),
        documentation_metadata(),
    ]
}

/// Get a builtin skill by name.
pub fn get_skill(name: &str) -> Option<Skill> {
    match name {
        "code-review" => Some(code_review()),
        "debugging" => Some(debugging()),
        "testing" => Some(testing()),
        "architecture-review" => Some(architecture_review()),
        "incident-response" => Some(incident_response()),
        "data-analysis" => Some(data_analysis()),
        "sql-queries" => Some(sql_queries()),
        "security-review" => Some(security_review()),
        "documentation" => Some(documentation()),
        _ => None,
    }
}

/// Get builtin skill metadata by name.
pub fn get_metadata(name: &str) -> Option<SkillMetadata> {
    match name {
        "code-review" => Some(code_review_metadata()),
        "debugging" => Some(debugging_metadata()),
        "testing" => Some(testing_metadata()),
        "architecture-review" => Some(architecture_review_metadata()),
        "incident-response" => Some(incident_response_metadata()),
        "data-analysis" => Some(data_analysis_metadata()),
        "sql-queries" => Some(sql_queries_metadata()),
        "security-review" => Some(security_review_metadata()),
        "documentation" => Some(documentation_metadata()),
        _ => None,
    }
}

// === Code Review ===

fn code_review_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "code-review".to_string(),
        description: "Review code changes for correctness, style, security, and best practices. \
            Use when reviewing pull requests, commits, or code snippets.".to_string(),
        category: Some(SkillCategory::Engineering),
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
    }
}

fn code_review() -> Skill {
    Skill {
        name: "code-review".to_string(),
        description: "Review code changes for correctness, style, security, and best practices. \
            Use when reviewing pull requests, commits, or code snippets.".to_string(),
        category: Some(SkillCategory::Engineering),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
        metadata: Default::default(),
        instructions: CODE_REVIEW_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const CODE_REVIEW_INSTRUCTIONS: &str = r#"# Code Review

## When to use this skill

Use this skill when:
- Reviewing pull requests or merge requests
- Analyzing code changes in commits
- Providing feedback on code snippets
- Checking code for issues before merging

## Review process

### 1. Understand the context
- Read the PR description and linked issues
- Understand the intent of the changes
- Check if the scope matches the description

### 2. Review for correctness
- Does the code do what it's supposed to do?
- Are there edge cases not handled?
- Are there potential bugs or logic errors?

### 3. Review for security
- Input validation and sanitization
- Authentication and authorization checks
- Sensitive data handling
- SQL injection, XSS, and other vulnerabilities

### 4. Review for maintainability
- Is the code readable and well-organized?
- Are functions and variables named clearly?
- Is there appropriate documentation?
- Does it follow project conventions?

### 5. Review for performance
- Are there obvious performance issues?
- N+1 queries, unnecessary loops, etc.
- Memory usage concerns

## Providing feedback

- Be specific and actionable
- Explain the "why" not just the "what"
- Distinguish between required changes and suggestions
- Acknowledge good code and clever solutions
- Use a constructive and respectful tone

## Example feedback format

```
**[Required]** The input is not validated before use.
Consider adding: `if (!isValid(input)) return error;`

**[Suggestion]** This loop could be simplified using `map()`.

**[Question]** What happens if `user` is null here?

**[Praise]** Nice use of the builder pattern here!
```
"#;

// === Debugging ===

fn debugging_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "debugging".to_string(),
        description: "Diagnose and fix bugs, errors, and unexpected behavior in code. \
            Use when investigating issues, analyzing stack traces, or troubleshooting problems.".to_string(),
        category: Some(SkillCategory::Engineering),
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
    }
}

fn debugging() -> Skill {
    Skill {
        name: "debugging".to_string(),
        description: "Diagnose and fix bugs, errors, and unexpected behavior in code. \
            Use when investigating issues, analyzing stack traces, or troubleshooting problems.".to_string(),
        category: Some(SkillCategory::Engineering),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
        metadata: Default::default(),
        instructions: DEBUGGING_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const DEBUGGING_INSTRUCTIONS: &str = r#"# Debugging

## When to use this skill

Use this skill when:
- Investigating error messages or stack traces
- Finding the root cause of unexpected behavior
- Troubleshooting failing tests
- Analyzing production incidents

## Debugging process

### 1. Reproduce the issue
- Understand the exact steps to reproduce
- Identify the expected vs actual behavior
- Note any error messages or logs

### 2. Gather information
- Read relevant error messages and stack traces
- Check logs for context
- Review recent changes that might be related

### 3. Form hypotheses
- Based on symptoms, what could be wrong?
- List possible causes in order of likelihood
- Consider recent changes, edge cases, external dependencies

### 4. Test hypotheses
- Add logging or debugging statements
- Isolate the problem by simplifying
- Check assumptions about data and state

### 5. Fix and verify
- Make the minimal fix needed
- Verify the fix solves the problem
- Check for regression in related functionality
- Add tests to prevent recurrence

## Common debugging patterns

### Null/undefined errors
- Check all object accesses for null
- Verify data is loaded before use
- Look for race conditions in async code

### Type errors
- Check function argument types
- Verify API response shapes
- Look for implicit type coercion

### Logic errors
- Trace through code with actual values
- Check boundary conditions
- Verify loop termination conditions

### Performance issues
- Profile to find hot spots
- Check for N+1 queries
- Look for unnecessary work in loops
"#;

// === Testing ===

fn testing_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "testing".to_string(),
        description: "Write and improve tests for code. \
            Use when creating unit tests, integration tests, or improving test coverage.".to_string(),
        category: Some(SkillCategory::Testing),
        allowed_tools: vec![
            "repo_read".to_string(),
            "repo_write".to_string(),
            "context_search".to_string(),
        ],
    }
}

fn testing() -> Skill {
    Skill {
        name: "testing".to_string(),
        description: "Write and improve tests for code. \
            Use when creating unit tests, integration tests, or improving test coverage.".to_string(),
        category: Some(SkillCategory::Testing),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "repo_write".to_string(),
            "context_search".to_string(),
        ],
        metadata: Default::default(),
        instructions: TESTING_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const TESTING_INSTRUCTIONS: &str = r#"# Testing

## When to use this skill

Use this skill when:
- Writing unit tests for new code
- Adding integration tests
- Improving test coverage
- Fixing or updating existing tests

## Testing principles

### Test structure (AAA pattern)
1. **Arrange**: Set up test data and preconditions
2. **Act**: Execute the code under test
3. **Assert**: Verify the expected outcome

### What to test
- Happy path: normal expected usage
- Edge cases: boundary values, empty inputs
- Error cases: invalid inputs, failures
- Security cases: malicious inputs

### Test naming
Use descriptive names that explain what's being tested:
- `test_create_user_with_valid_email_succeeds`
- `test_create_user_with_invalid_email_returns_error`
- `test_delete_user_removes_associated_data`

## Unit test guidelines

- Test one thing per test
- Tests should be independent
- Avoid testing implementation details
- Mock external dependencies
- Keep tests fast

## Integration test guidelines

- Test real interactions between components
- Use test databases or containers
- Clean up test data after runs
- Test API contracts

## Example test structure

```rust
#[test]
fn test_calculate_total_with_discount() {
    // Arrange
    let cart = Cart::new();
    cart.add_item(Item { price: 100 });
    let discount = Discount::percentage(10);

    // Act
    let total = cart.calculate_total(Some(discount));

    // Assert
    assert_eq!(total, 90);
}
```
"#;

// === Architecture Review ===

fn architecture_review_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "architecture-review".to_string(),
        description: "Review and design software architecture decisions. \
            Use when evaluating system design, reviewing RFCs, or planning technical changes.".to_string(),
        category: Some(SkillCategory::Engineering),
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
    }
}

fn architecture_review() -> Skill {
    Skill {
        name: "architecture-review".to_string(),
        description: "Review and design software architecture decisions. \
            Use when evaluating system design, reviewing RFCs, or planning technical changes.".to_string(),
        category: Some(SkillCategory::Engineering),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
        metadata: Default::default(),
        instructions: ARCHITECTURE_REVIEW_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const ARCHITECTURE_REVIEW_INSTRUCTIONS: &str = r#"# Architecture Review

## When to use this skill

Use this skill when:
- Reviewing system design documents or RFCs
- Evaluating architectural decisions
- Planning major technical changes
- Assessing technical debt

## Review dimensions

### Scalability
- Can the system handle growth?
- What are the bottlenecks?
- How does it scale horizontally/vertically?

### Reliability
- What happens when components fail?
- Are there single points of failure?
- What's the disaster recovery plan?

### Maintainability
- Is the architecture easy to understand?
- Can components be updated independently?
- Is there appropriate separation of concerns?

### Security
- What's the attack surface?
- How is data protected?
- What are the trust boundaries?

### Cost
- What are the infrastructure costs?
- Are there cost optimization opportunities?
- What's the build vs buy analysis?

## Architecture patterns to consider

- Microservices vs monolith
- Event-driven vs request-response
- Synchronous vs asynchronous
- Caching strategies
- Data consistency models
"#;

// === Incident Response ===

fn incident_response_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "incident-response".to_string(),
        description: "Handle and resolve production incidents. \
            Use when responding to outages, investigating issues, or coordinating incident response.".to_string(),
        category: Some(SkillCategory::IncidentResponse),
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
    }
}

fn incident_response() -> Skill {
    Skill {
        name: "incident-response".to_string(),
        description: "Handle and resolve production incidents. \
            Use when responding to outages, investigating issues, or coordinating incident response.".to_string(),
        category: Some(SkillCategory::IncidentResponse),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "web_fetch".to_string(),
        ],
        metadata: Default::default(),
        instructions: INCIDENT_RESPONSE_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const INCIDENT_RESPONSE_INSTRUCTIONS: &str = r#"# Incident Response

## When to use this skill

Use this skill when:
- Responding to production incidents or outages
- Investigating system issues
- Coordinating incident response
- Writing post-incident reviews

## Incident response process

### 1. Triage
- Assess severity and impact
- Identify affected systems and users
- Establish communication channels

### 2. Mitigate
- Focus on reducing impact first
- Consider rollbacks, feature flags, or scaling
- Don't debug in production during an outage

### 3. Investigate
- Gather logs, metrics, and traces
- Build a timeline of events
- Identify root cause

### 4. Resolve
- Implement fix or workaround
- Verify resolution
- Monitor for recurrence

### 5. Follow up
- Write incident report
- Identify action items
- Schedule post-mortem

## Severity levels

- **P0/Critical**: Complete outage, major data loss
- **P1/High**: Significant degradation, many users affected
- **P2/Medium**: Partial issues, workarounds available
- **P3/Low**: Minor issues, limited impact
"#;

// === Data Analysis ===

fn data_analysis_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "data-analysis".to_string(),
        description: "Analyze datasets, create visualizations, and generate insights. \
            Use when exploring data, creating reports, or answering data questions.".to_string(),
        category: Some(SkillCategory::DataAnalysis),
        allowed_tools: vec![
            "db_query".to_string(),
            "data_export".to_string(),
            "context_search".to_string(),
        ],
    }
}

fn data_analysis() -> Skill {
    Skill {
        name: "data-analysis".to_string(),
        description: "Analyze datasets, create visualizations, and generate insights. \
            Use when exploring data, creating reports, or answering data questions.".to_string(),
        category: Some(SkillCategory::DataAnalysis),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "db_query".to_string(),
            "data_export".to_string(),
            "context_search".to_string(),
        ],
        metadata: Default::default(),
        instructions: DATA_ANALYSIS_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const DATA_ANALYSIS_INSTRUCTIONS: &str = r#"# Data Analysis

## When to use this skill

Use this skill when:
- Exploring and understanding datasets
- Answering business questions with data
- Creating reports and visualizations
- Identifying trends and patterns

## Analysis process

### 1. Understand the question
- What decision will this analysis inform?
- What's the business context?
- What level of precision is needed?

### 2. Explore the data
- Check data quality and completeness
- Understand the schema and relationships
- Look for outliers and anomalies

### 3. Analyze
- Apply appropriate statistical methods
- Consider multiple perspectives
- Validate assumptions

### 4. Communicate
- Present findings clearly
- Include caveats and limitations
- Provide actionable recommendations
"#;

// === SQL Queries ===

fn sql_queries_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "sql-queries".to_string(),
        description: "Write and optimize SQL queries. \
            Use when querying databases, writing reports, or optimizing query performance.".to_string(),
        category: Some(SkillCategory::DataAnalysis),
        allowed_tools: vec![
            "db_query".to_string(),
            "context_search".to_string(),
        ],
    }
}

fn sql_queries() -> Skill {
    Skill {
        name: "sql-queries".to_string(),
        description: "Write and optimize SQL queries. \
            Use when querying databases, writing reports, or optimizing query performance.".to_string(),
        category: Some(SkillCategory::DataAnalysis),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "db_query".to_string(),
            "context_search".to_string(),
        ],
        metadata: Default::default(),
        instructions: SQL_QUERIES_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const SQL_QUERIES_INSTRUCTIONS: &str = r#"# SQL Queries

## When to use this skill

Use this skill when:
- Writing database queries
- Optimizing slow queries
- Creating reports from database
- Debugging data issues

## Best practices

### Query structure
- Use explicit column names, not SELECT *
- Use meaningful table aliases
- Format queries for readability
- Add comments for complex logic

### Performance
- Use indexes effectively
- Avoid SELECT DISTINCT when possible
- Use EXPLAIN to understand query plans
- Limit result sets appropriately

### Security
- Use parameterized queries
- Never concatenate user input
- Validate and sanitize inputs
"#;

// === Security Review ===

fn security_review_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "security-review".to_string(),
        description: "Review code and systems for security vulnerabilities. \
            Use when auditing code, checking for vulnerabilities, or assessing security posture.".to_string(),
        category: Some(SkillCategory::Security),
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "security_scan".to_string(),
        ],
    }
}

fn security_review() -> Skill {
    Skill {
        name: "security-review".to_string(),
        description: "Review code and systems for security vulnerabilities. \
            Use when auditing code, checking for vulnerabilities, or assessing security posture.".to_string(),
        category: Some(SkillCategory::Security),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "context_search".to_string(),
            "security_scan".to_string(),
        ],
        metadata: Default::default(),
        instructions: SECURITY_REVIEW_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const SECURITY_REVIEW_INSTRUCTIONS: &str = r#"# Security Review

## When to use this skill

Use this skill when:
- Reviewing code for security issues
- Conducting security audits
- Assessing vulnerability reports
- Checking security configurations

## Common vulnerabilities (OWASP Top 10)

### Injection
- SQL injection
- Command injection
- LDAP injection

### Authentication/Authorization
- Broken authentication
- Broken access control
- Missing function-level access control

### Data exposure
- Sensitive data exposure
- Security misconfiguration
- Missing encryption

### Other
- Cross-site scripting (XSS)
- Cross-site request forgery (CSRF)
- Insecure deserialization

## Review checklist

- [ ] Input validation on all user input
- [ ] Output encoding for XSS prevention
- [ ] Parameterized queries for SQL
- [ ] Authentication on sensitive endpoints
- [ ] Authorization checks for resources
- [ ] Secrets not in code or logs
- [ ] HTTPS enforced
- [ ] Security headers configured
"#;

// === Documentation ===

fn documentation_metadata() -> SkillMetadata {
    SkillMetadata {
        name: "documentation".to_string(),
        description: "Write and improve technical documentation. \
            Use when creating READMEs, API docs, or technical guides.".to_string(),
        category: Some(SkillCategory::Documentation),
        allowed_tools: vec![
            "repo_read".to_string(),
            "repo_write".to_string(),
            "context_search".to_string(),
        ],
    }
}

fn documentation() -> Skill {
    Skill {
        name: "documentation".to_string(),
        description: "Write and improve technical documentation. \
            Use when creating READMEs, API docs, or technical guides.".to_string(),
        category: Some(SkillCategory::Documentation),
        license: Some("Apache-2.0".to_string()),
        compatibility: None,
        allowed_tools: vec![
            "repo_read".to_string(),
            "repo_write".to_string(),
            "context_search".to_string(),
        ],
        metadata: Default::default(),
        instructions: DOCUMENTATION_INSTRUCTIONS.to_string(),
        scripts: vec![],
        references: vec![],
        assets: vec![],
    }
}

const DOCUMENTATION_INSTRUCTIONS: &str = r#"# Documentation

## When to use this skill

Use this skill when:
- Writing README files
- Creating API documentation
- Writing technical guides
- Documenting architecture decisions

## Documentation types

### README
- Project overview
- Quick start guide
- Installation instructions
- Basic usage examples

### API documentation
- Endpoint descriptions
- Request/response examples
- Authentication details
- Error codes

### Technical guides
- Architecture overviews
- Setup guides
- Troubleshooting guides
- Migration guides

## Best practices

- Write for your audience
- Include concrete examples
- Keep it up to date
- Use consistent formatting
- Include a table of contents for long docs
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_metadata() {
        let metadata = all_metadata();
        assert!(!metadata.is_empty());
        assert!(metadata.len() >= 9);
    }

    #[test]
    fn test_get_skill() {
        let skill = get_skill("code-review");
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.name, "code-review");
        assert!(!skill.instructions.is_empty());
    }

    #[test]
    fn test_get_metadata() {
        let metadata = get_metadata("debugging");
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert_eq!(metadata.name, "debugging");
    }

    #[test]
    fn test_unknown_skill() {
        assert!(get_skill("unknown-skill").is_none());
        assert!(get_metadata("unknown-skill").is_none());
    }

    #[test]
    fn test_skill_has_instructions() {
        for metadata in all_metadata() {
            let skill = get_skill(&metadata.name);
            assert!(skill.is_some(), "Skill {} not found", metadata.name);
            let skill = skill.unwrap();
            assert!(
                !skill.instructions.is_empty(),
                "Skill {} has no instructions",
                metadata.name
            );
        }
    }
}
