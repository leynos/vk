# A Comprehensive Guide to End-to-End Testing for the `vk` Command-Line Tool

<!-- markdownlint-disable MD033 MD038 -->

## A Primer on End-to-End Testing for the vk CLI

This document provides a definitive, expert-level guide to establishing a
comprehensive end-to-end (E2E) testing strategy for the `vk` command-line
interface (CLI). It transforms the foundational proposal into a practical,
actionable report, detailing the methodology, tools, and best practices
required to ensure the tool's long-term reliability, correctness, and
maintainability.

### Introduction to vk: Viewing Komments from the Terminal

The `vk` (View Komments) tool is a command-line utility written in Rust,
designed to streamline the developer code review workflow. Its primary function
is to fetch and display unresolved code review comments from GitHub pull
requests and issues directly within the terminal. The project's author is a
Staff Engineer at GitHub, lending significant domain credibility to the tool's
design and its focus on optimizing interactions within the GitHub ecosystem.1

A core feature of `vk` is its commitment to a rich user experience, achieved
through the use of the `termimad` crate.2 This library enables the rendering of
formatted text, including Markdown, with colours, tables, and other styling
elements directly in the terminal.3 This focus on richly formatted output is a
key consideration for testing, as simple string comparisons are inadequate to
validate the visual correctness of the tool's output.

### The Imperative for E2E Testing in Network-Dependent CLIs

End-to-end testing, in the context of a command-line application, involves
treating the final compiled binary as a "black box".5 The test suite interacts
with the application solely through its public interface—command-line
arguments, environment variables, and standard input—and verifies its output,
exit code, and standard error streams. This approach simulates how a real user
interacts with the tool, providing the highest level of confidence in its
overall functionality.

For a tool like `vk`, a robust E2E testing strategy is not merely beneficial;
it is essential for several critical reasons:

- **Reliability:** The tool's fundamental purpose is to communicate with an
  external network service, the GitHub GraphQL API. While unit tests can verify
  individual functions, only E2E tests can validate the entire sequence of
  operations: parsing user input, constructing the correct API request,
  handling the response, and rendering the output.

- **Correctness:** The `vk` tool must correctly interpret a variety of user
  inputs, formulate valid GraphQL queries, parse the JSON responses, and
  translate that data into a formatted terminal display. E2E tests are uniquely
  positioned to verify this entire chain of correctness.

- **Regression Prevention:** As `vk` evolves with new features or bug fixes, a
  comprehensive E2E test suite acts as a critical safety net. It ensures that
  modifications in one area do not inadvertently break existing functionality
  in another, a cornerstone of maintainable software development.6

- **User Experience (UX) Validation:** The use of `termimad` signifies that the
  visual presentation of data is a primary feature.7 The colours, layout, and
  formatting are integral to the tool's value. E2E tests, when combined with
  snapshot testing, provide the only effective means to validate this complex,
  styled output and prevent visual regressions.

The entire testing strategy outlined in this guide is built upon the principle
of **hermetic testing**. A hermetic test suite is one that is self-contained
and completely isolated from external dependencies. It does not rely on network
connectivity, the live state of the GitHub API, or the configuration of the
machine on which it runs. This isolation is the key to creating a test suite
that is fast, deterministic, and free from the "flakiness" that often plagues
tests with external dependencies. By achieving this hermetic state, the tests
can be run reliably anywhere, from a developer's local machine to a continuous
integration (CI) pipeline.

### Architectural Overview of the Chosen Testing Stack

To achieve a hermetic and comprehensive E2E testing environment for `vk`, this
guide employs a carefully selected "testing triad" of Rust crates. Each
component serves a distinct and complementary purpose, working in concert to
provide a holistic solution.

- `assert_cmd`: This crate serves as the test orchestrator. It is responsible
  for invoking the compiled `vk` binary, simulating user input by providing
  command-line arguments and environment variables, and performing assertions
  on the process-level results. This includes checking the process exit code
  for success or failure and inspecting the contents of the standard error
  (`stderr`) stream for user-facing error messages.8

- `third-wheel`: This crate provides the critical network isolation layer. It
  is a Man-in-the-Middle (MITM) proxy, written in Rust, that can be embedded
  directly within the test harness.11 Its role is to intercept all outgoing
  HTTP requests that

  `vk` attempts to make to the GitHub GraphQL API. Instead of allowing these
  requests to reach the internet, `third-wheel` captures them and returns
  controlled, predefined responses from local fixture files. This makes the
  tests completely independent of the network and ensures that the API's
  behavior is deterministic for every test run.

- `insta`: This crate is the output verifier, specialized for snapshot testing.
  Given that `vk` produces complex, styled terminal output via `termimad`,
  `insta` is used to capture this raw output—including all ANSI escape codes
  for colour and formatting—and save it to a "snapshot" file.12 On subsequent
  test runs,

  `insta` compares the new output against the saved snapshot. Any deviation
  will cause the test to fail, allowing developers to either fix the regression
  or intentionally update the snapshot to reflect a desired change. This
  approach is vastly superior to manual string assertions for validating rich
  UIs.14

Together, these three tools form a powerful and cohesive system. `assert_cmd`
drives the application, `third-wheel` controls its external environment, and
`insta` verifies its final output. This architecture enables the creation of a
test suite that is robust, maintainable, and provides the highest degree of
confidence in the correctness of the `vk` CLI.

## Establishing the Test Harness

A well-defined test harness is the foundation of a maintainable and effective
test suite. This section details the necessary steps to configure the project,
organize test files, and create an initial smoke test to validate the setup.

### Configuring Cargo.toml for a Robust Test Suite

In a Rust project, dependencies required only for testing, benchmarking, or
examples are placed in the `[dev-dependencies]` section of the `Cargo.toml`
file.15 This ensures that testing libraries are not compiled into the final
release binary, keeping it lean and free of unnecessary code.

The following dependencies are required to build the complete E2E test suite
for `vk`. Each one plays a specific, interconnected role within the test
architecture. For instance, the `third-wheel` mock server is asynchronous and
therefore requires the `tokio` runtime to execute. The mock server, in turn,
needs to serve predefined JSON responses, which are loaded and handled using
`serde_json`. This interconnectedness highlights how the chosen stack forms a
self-contained ecosystem for testing.

To set up the project, add the following `[dev-dependencies]` section to the
`Cargo.toml` file:

Ini, TOML

```
[dev-dependencies]
assert_cmd = "2.0"
insta = { version = "1.34", features = ["redactions"] }
third-wheel = "0.6"
tokio = { version = "1.0", features = ["full"] }
serde_json = "1.0"
tempfile = "3.8"
```

The table below outlines the purpose of each dependency within the test suite.

| Crate       | Recommended Version | Purpose in Test Suite                                                                                                                                |
| ----------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| assert_cmd  | ~2.0                | The core test orchestrator for executing the vk binary and asserting on its behaviour.8                                                              |
| insta       | ~1.34               | For snapshot testing of the styled terminal output, handling the complexity of termimad.17 The redactions feature is enabled to handle dynamic data. |
| third-wheel | ~0.6                | An embedded MITM proxy to intercept and mock GitHub API calls, ensuring deterministic tests.18                                                       |
| tokio       | ~1.0                | An async runtime required to run the third-wheel mock server concurrently with the test logic. The full feature flag is recommended for simplicity.  |
| serde_json  | ~1.0                | A utility for loading and manipulating the JSON fixture files used as mock API responses.                                                            |
| tempfile    | ~3.8                | For creating temporary configuration files and directories to test vk's configuration logic in an isolated manner.19                                 |

### Test File Organization: Following Rust Conventions

maintain. Rust has a well-established convention for test organization:
integration tests are placed in a top-level `tests/` directory, which resides
alongside the `src/` directory.16 Each Rust file (

`.rs`) within the `tests/` directory is compiled and run as a separate test
crate.

For the `vk` project, the following structure is recommended:

```
vk/
├── Cargo.toml
├── src/
│   └── main.rs
└── tests/
    ├── e2e.rs
    └── fixtures/
        └── pr_123_comments.json
```

- `tests/e2e.rs`: This file will contain the end-to-end tests. Naming it
  `e2e.rs` clearly communicates its purpose and distinguishes these tests from
  any unit tests that might exist within the `src/` directory.

- `tests/fixtures/`: This directory will house the JSON files used as mock API
  responses for the `third-wheel` server. Separating test data (fixtures) from
  test code (`e2e.rs`) is a crucial practice for keeping the test suite clean
  and organized.

- `pr_123_comments.json`: An example fixture file containing a valid JSON
  response from the GitHub GraphQL API for a specific pull request.

As the test suite grows, it can be further organized by creating submodules.
For example, tests related to configuration could be moved to `tests/config.rs`
and command-specific tests to `tests/pr_commands.rs`. This modular approach,
supported natively by Cargo's test runner, is a powerful pattern for
maintaining large test suites.20

### A "Hello World" Test: The Foundational Smoke Test

Before building complex tests involving API mocking, it is essential to create
a simple "smoke test." This foundational test verifies that the basic test
harness is configured correctly and that the test runner can locate and execute
the `vk` binary. A perfect candidate for this is testing the `--help` flag.

The following code should be placed in `tests/e2e.rs`. It uses `assert_cmd` to
run `vk --help` and asserts that the command executes successfully and that its
output contains expected text.

```rust
// tests/e2e.rs
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_help_output_is_displayed_successfully() {
    // Command::cargo_bin("vk") is the canonical way to create a Command
    // instance that points to the binary of the crate being tested.
    // It automatically finds the correct binary in the target directory. [8, 22]
    let mut cmd = Command::cargo_bin("vk").unwrap();

    // Use the.arg() method to pass a command-line argument.
    cmd.arg("--help");

    // The.assert() method executes the command and returns an Assert
    // object, which provides a fluent interface for making assertions.
    cmd.assert()
        //.success() asserts that the process exited with a status code of 0. [23]
       .success()
        //.stdout() asserts on the content of the standard output.
        // Here, we use a predicate to check that the output contains the string "Usage: vk".
        // Predicates provide more flexible matching than simple equality. [15]
       .stdout(predicate::str::contains("Usage: vk"));
}
```

To run this test, execute `cargo test` in the terminal. A successful run of
this test provides high confidence that:

1. The `[dev-dependencies]` are correctly configured.

2. Cargo's test runner can find and compile the `tests/e2e.rs` file.

3. `assert_cmd` is able to locate the `vk` binary produced by the build process.

4. The basic assertion mechanism is working as expected.

With this foundation in place, the next step is to introduce the complexity of
API mocking.

## Deterministic API Behavior via Mocking with third-wheel

To create a truly hermetic test suite for a network-dependent application like
`vk`, it is imperative to isolate it from the actual network. Making live calls
to the GitHub API during tests is untenable; it would make the tests slow,
flaky (dependent on network conditions and API availability), and would require
valid authentication tokens, posing a security risk in CI environments. The
solution is to mock the API.

### The Role of a Man-in-the-Middle (MITM) Proxy in Testing

While one could attempt to mock the `GraphQLClient` struct within `vk`'s source
code, this approach has significant drawbacks for E2E testing. It would require
modifying the application code specifically for testing (e.g., with
`#[cfg(test)]` attributes), which moves away from true black-box testing.

A superior approach is to use a Man-in-the-Middle (MITM) proxy. This technique
involves placing a server *between* the application under test and the real API
endpoint. This proxy transparently intercepts all outgoing network traffic. For
testing, this allows us to capture requests intended for `api.github.com` and
return a controlled, deterministic response without the request ever leaving
the local machine. This method requires zero changes to the `vk` source code,
preserving the integrity of the black-box testing model.

The `third-wheel` crate is ideal for this purpose because it is a lightweight
MITM proxy written in Rust.11 This allows it to be embedded and controlled
programmatically from within the test code itself, eliminating the complexity
of managing a separate, external proxy process.11

### Step-by-Step: Embedding the third-wheel Mock Server

Since `third-wheel` is an asynchronous server, any test that uses it must be
executed within an asynchronous runtime. The `tokio` crate is the de facto
standard for this in the Rust ecosystem. By annotating a test with
`#[tokio::test]`, the test function becomes an `async` function capable of
running asynchronous code, such as starting and managing the mock server.

The following code demonstrates a reusable helper function,
`start_mock_server`, which encapsulates the logic for setting up and running
the `third-wheel` proxy.

```rust
// In tests/e2e.rs, or a new tests/helpers.rs module

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use third_wheel::{ThirdWheel, MitmProxy};
use third_wheel::hyper::{Body, Request, Response, StatusCode};

// This type alias simplifies the handler signature. The handler will be
// a shared, mutable function that determines the mock server's behavior.
type Handler = Arc<Mutex<Box<dyn Fn(&Request<Body>) -> Response<Body> + Send>>>;

/// Starts a third-wheel mock server on a random available port.
///
/// The server's behavior is defined by the `handler` closure. It returns the
/// server's address and a clone of the handler so the test can dynamically
/// change the mock response.
async fn start_mock_server() -> (SocketAddr, Handler) {
    // Define a default handler that returns a 404 Not Found for any request.
    // This is a safe default to ensure tests fail if they don't specify a mock response.
    let handler: Handler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
           .status(StatusCode::NOT_FOUND)
           .body(Body::from("Mock response not configured for this request."))
           .unwrap()
    })));

    let handler_clone = handler.clone();

    // The MitmProxy requires a function that will be called for each intercepted request.
    let proxy = MitmProxy::new(Box::new(move |req, _ctx| {
        // We lock the mutex to access the current handler function and execute it.
        // This allows the test to change the server's behavior on the fly.
        let h = handler_clone.lock().unwrap();
        h(req)
    }));

    // Bind the server to port 0, which tells the OS to assign a random available port.
    // This is crucial for running tests in parallel without port conflicts.
    let (addr, wheel) = ThirdWheel::new(Box::new(proxy))
       .bind("127.0.0.1:0".parse().unwrap())
       .await
       .unwrap();

    // Spawn the server in a separate tokio task so it runs in the background
    // without blocking the main test thread.
    tokio::spawn(wheel);

    (addr, handler)
}
```

This helper function provides a powerful and flexible foundation. A test can
call it to get a running server, and then modify the `handler` to define the
exact response needed for that specific test case. This programmatic approach
to mocking is a key advantage of the `third-wheel` library, as it avoids the
need for complex, static configuration files and allows for highly isolated and
readable tests.

### Managing and Using Mock API Fixtures

To simulate realistic API responses, the mock server will serve content from
JSON files stored in the `tests/fixtures/` directory. These files contain
actual, valid responses captured from the GitHub GraphQL API.

**Example Fixture:** `tests/fixtures/pr_with_comments.json`

```json
{
  "data": {
    "repository": {
      "pullRequest": {
        "url": "https://github.com/leynos/vk/pull/1",
        "title": "Add initial implementation",
        "reviewThreads": {
          "nodes":
              }
            }
          ]
        }
      }
    }
  }
}
```

A simple helper function can be used to load these fixtures from disk:

```rust
// In tests/e2e.rs or helpers.rs
use std::fs;
use std::path::Path;

/// Loads a fixture file from the `tests/fixtures` directory.
fn load_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
       .join("tests/fixtures")
       .join(name);
    fs::read_to_string(path).expect("Failed to load fixture file")
}
```

### Overriding the API Endpoint

With the mock server running, the final step is to instruct `vk` to send its
API requests to our mock server instead of the real GitHub API. The `vk` tool
is designed to be testable and respects the `GITHUB_GRAPHQL_URL` environment
variable to override the default API endpoint. The `assert_cmd` crate makes
setting this variable for the child process trivial using the `.env()` method.

### Simulating Diverse API Scenarios

The true power of this embedded mocking approach is the ability to easily
simulate a wide range of API behaviors to test `vk`'s resilience and error
handling.

Scenario 1: Successful Query

The test configures the handler to return the contents of a fixture with a 200
OK status.

```rust
// Inside a #[tokio::test] function
let (addr, handler) = start_mock_server().await;
let mock_response = load_fixture("pr_with_comments.json");

*handler.lock().unwrap() = Box::new(move |_req| {
    Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(mock_response.clone()))
       .unwrap()
});

//... run vk command with.env("GITHUB_GRAPHQL_URL", format!("http://{}", addr))...
```

Scenario 2: GraphQL Error Response

GraphQL APIs typically return HTTP 200 OK even when the query contains errors,
placing the error details inside a JSON errors object. The mock server can
simulate this perfectly.

```rust
// Inside a #[tokio::test] function
let (addr, handler) = start_mock_server().await;
let error_response = r#"{
    "data": null,
    "errors":
    }]
}"#;

*handler.lock().unwrap() = Box::new(move |_req| {
    Response::builder()
       .status(StatusCode::OK)
       .header("Content-Type", "application/json")
       .body(Body::from(error_response))
       .unwrap()
});

//... run vk command and assert that stderr contains the GraphQL error message...
```

Scenario 3: Network Error

To test how vk handles network-level failures, the handler can be configured to
return an HTTP error status code like 503 Service Unavailable.

```rust
// Inside a #[tokio::test] function
let (addr, handler) = start_mock_server().await;

*handler.lock().unwrap() = Box::new(|_req| {
    Response::builder()
       .status(StatusCode::SERVICE_UNAVAILABLE)
       .body(Body::from("The service is temporarily unavailable."))
       .unwrap()
});

//... run vk command and assert that it fails with a network error message...
```

This level of programmatic control over the mock API's behavior is what enables
the creation of a truly comprehensive and robust E2E test suite, capable of
verifying not just the "happy path" but also a wide variety of failure modes.

## Driving the Application with assert_cmd

The `assert_cmd` crate is the engine of the E2E test suite, responsible for
executing the `vk` binary and simulating all forms of user interaction. Its
fluent, expressive API simplifies the process of setting up command-line
arguments, configuring the environment, and asserting on the results.9

### Executing the vk Binary

The most reliable way to instantiate a command for the crate under test is
`Command::cargo_bin("vk")`.8 This function, provided by the

`CommandCargoExt` trait, asks Cargo for the location of the specified binary
artifact. This approach is vastly superior to hardcoding a path like
`target/debug/vk`, as it is resilient to changes in target architecture, build
profiles (debug vs. release), and workspace layout. The `.unwrap()` is
typically used here, as in a test context, a failure to find the binary is a
critical, unrecoverable error that should cause a panic.

```rust
use assert_cmd::Command;

// This creates a Command struct ready to be configured and executed.
let mut cmd = Command::cargo_bin("vk").unwrap();
```

### Simulating User Input: Arguments and Environment

A key function of E2E testing is to verify that the application responds
correctly to the full range of inputs a user can provide. `assert_cmd` provides
simple methods for this.

Command-Line Arguments:

The .arg() method adds a single argument, while .args() adds a collection of
arguments.22 This allows for testing simple commands as well as those with
multiple flags and values.

```rust
// Simulating `vk pr 123`
cmd.arg("pr").arg("123");

// Simulating `vk pr https://github.com/org/repo/pull/123`
cmd.args(&["pr", "https://github.com/org/repo/pull/123"]);
```

Environment Variables:

The .env() method is used to set environment variables for the child process.8
This is critical for the

`vk` test suite, as it is used to provide the mock API URL and to test
configuration options like `GITHUB_TOKEN` and `VK_REPO`.24

```rust
// Provide a mock authentication token.
cmd.env("GITHUB_TOKEN", "fake_token_for_testing");

// Specify the repository via an environment variable.
cmd.env("VK_REPO", "leynos/vk");

// Redirect the application to the mock API server.
let mock_server_url = format!("http://{}", addr);
cmd.env("GITHUB_GRAPHQL_URL", mock_server_url);
```

The `.env_clear()` method can also be used to ensure the child process starts
with a clean environment, preventing variables from the test runner's
environment from leaking into the test and causing non-deterministic behavior.

### Asserting on Process Outcomes

A command-line tool communicates its result through two primary channels: its
exit code and its output to the standard error stream (`stderr`). `assert_cmd`
provides a powerful assertion API to validate both.

The chain begins with `.assert()`, which executes the command and returns an
`Assert` struct.

Asserting on Exit Codes:

The most common assertions relate to the success or failure of the command.

- `.success()`: Asserts that the process exited with a code of 0, indicating a
  successful operation.

- `.failure()`: Asserts that the process exited with a non-zero code,
  indicating that an error occurred.

- `.code(N)`: Asserts that the process exited with a specific integer code `N`.
  This is useful for testing applications that use different exit codes to
  signify different types of errors.23

```rust
// Assert that the command completed successfully.
cmd.assert().success();

// Assert that the command failed as expected.
cmd.assert().failure().code(1);
```

Asserting on Standard Error (stderr):

When a CLI tool fails, it should print a helpful, human-readable error message
to stderr. It is crucial to test that these messages are correct and
informative. assert_cmd integrates with the predicates crate to allow for
flexible string matching on stderr.

```rust
use predicates::prelude::*;

// Command to test an invalid input.
let mut cmd = Command::cargo_bin("vk").unwrap();
cmd.arg("pr").arg("this-is-not-a-valid-url");

// Assert that the command fails and prints a specific error to stderr.
cmd.assert()
   .failure()
   .stderr(predicate::str::contains("Error: Invalid pull request reference"));
```

Using `predicate::str::contains` is often preferable to an exact match
(`predicate::eq`), as it makes the test less brittle to minor changes in error
message formatting (e.g., changes in capitalization or punctuation).

By combining these methods, a developer can precisely control the conditions
under which `vk` is run and rigorously validate that it behaves as expected,
both in successful scenarios and in a wide variety of error cases.

## Verifying Rich Terminal Output with insta Snapshot Testing

While `assert_cmd` is excellent for verifying process outcomes and `stderr`, it
is not well-suited for validating the complex output `vk` prints to standard
output (`stdout`). Because `vk` uses the `termimad` crate, its output is not
plain text; it is a rich tapestry of content formatted with ANSI escape codes
that control colours, bolding, table layouts, and more.2

### The Case for Snapshot Testing with termimad

Attempting to validate this styled output with a traditional assertion like
`assert_eq!` would be a maintenance nightmare. A developer would have to
manually construct and hardcode strings containing cryptic escape sequences,
like `\u{1b} A snapshot testing library like `insta\` works by capturing the
*entire raw output* of a command on its first run and saving it to a dedicated
file (a "snapshot"). On all subsequent runs, the new output is compared to this
saved snapshot. If there is any difference, the test fails, and the library
presents a "diff" that clearly highlights the changes.13 This allows the
developer to:

1. Quickly identify an unintended change (a regression).

2. Consciously "approve" an intentional change, causing the snapshot to be
   updated with the new, correct output.

### Integrating insta with assert_cmd

The integration between `assert_cmd` and `insta` is seamless. The `stdout`
captured by `assert_cmd` is a vector of bytes (`Vec<u8>`), which can be easily
converted to a string and passed directly to an `insta` assertion macro.

The primary macro for this purpose is `insta::assert_snapshot!`. It takes the
value to be tested and compares it against a snapshot file, which it manages
automatically.

```rust
// Inside a test function...
let mut cmd = Command::cargo_bin("vk").unwrap();
//... configure cmd with args and env vars...

// Execute the command and capture its output.
let output = cmd.output().unwrap();

// Convert the raw stdout bytes to a UTF-8 string.
let stdout = String::from_utf8(output.stdout).unwrap();

// Assert that the stdout matches the stored snapshot.
// insta will automatically name the snapshot based on the test module and function name.
insta::assert_snapshot!(stdout);
```

### The Snapshot Lifecycle: Review and Update

The development workflow with `insta` is designed to be interactive and
intuitive, revolving around the `cargo-insta` command-line tool.13

1. **First Run and Snapshot Creation:** When a test containing
   `insta::assert_snapshot!` is run for the first time, there is no existing
   snapshot to compare against. The test will fail with a message like
   `Error: New snapshot 'e2e__my_test_name' created.` Simultaneously, `insta`
   creates a new file, for example, `tests/snapshots/e2e__my_test_name.snap`.
   This file contains the raw `stdout` captured during the test run.

2. **Reviewing the Snapshot:** The developer's next step is to open the newly
   created `.snap` file and inspect its contents. This is the "approval" step
   of approval testing. The developer verifies that the output, including all
   styling, is correct.

3. **Interactive Review and Acceptance with** `cargo insta review`**:** Instead
   of manually managing files, the recommended workflow is to use the
   interactive review tool. After running the tests, the developer runs
   `cargo insta review`. This tool will find all pending (new or changed)
   snapshots and present them one by one with a colourful diff. The developer
   has several options for each snapshot 13:

   - **Accept (**`a` **or** `Enter`**):** Approves the new snapshot. `insta`
     will update the `.snap` file (or remove the `.new` extension).

   - **Reject (**`r` **or** `Escape`**):** Rejects the change. `insta` will
     delete the pending snapshot file, and the test will continue to fail until
     the code is fixed.

   - **Skip (**`s` **or** `Space`**):** Skips reviewing this snapshot for now,
     leaving it in a pending state.

4. **Non-Interactive Updates (for CI/CD):** The `INSTA_UPDATE` environment
   variable controls `insta`'s behavior in non-interactive environments like CI
   pipelines.13

   - `INSTA_UPDATE=no`: This is the default behavior in most CI environments.
     If a snapshot mismatch is found, the test fails, and no files are written.
     This is the correct setting for CI, as it should only verify, not update,
     tests.

   - `INSTA_UPDATE=always`: This mode will cause `insta` to automatically
     overwrite any mismatched snapshots with the new output. This can be useful
     for bulk-updating many snapshots after a large, intentional change, but it
     should be used with extreme caution as it bypasses the crucial review step.

   - `INSTA_UPDATE=new`: This is the default for local runs. It writes new or
     changed snapshots to files with a `.new` extension, marking them as
     pending for review with `cargo insta review`.

### Handling Non-Deterministic Data with Redactions

Sometimes, application output contains data that changes on every run, such as
timestamps, unique identifiers, or performance measurements. This
non-determinism would cause snapshot tests to fail on every execution. `insta`
provides a powerful solution for this: **redactions**.13

Redactions allow you to specify patterns to be replaced with a static
placeholder before the comparison happens. This is done by providing a second
argument to the assertion macro.

Suppose `vk`'s output included a timestamp of when the data was fetched, like
`Fetched at: 2023-10-27T10:00:00Z`. This would break the snapshot on every run.
A redaction can be used to stabilize it:

```rust
let output = cmd.output().unwrap();
let stdout = String::from_utf8(output.stdout).unwrap();

// The second argument to the macro is a map-like expression for redactions.
insta::assert_snapshot!(stdout, {
    // The key is the static placeholder that will appear in the snapshot.
    // The value is a regular expression that matches the dynamic data.
    "[timestamp]" => r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"
});
```

When this test runs, any string matching the regex (e.g.,
`2023-10-27T10:00:00Z`) will be replaced with the literal string `[timestamp]`
before being compared to the snapshot. The `.snap` file itself will contain
`[timestamp]`, making the test deterministic and robust against changing data.

## A Complete E2E Test Case: From Arrangement to Assertion

This section synthesizes all the concepts from the preceding sections into a
single, fully-worked example. It provides a complete, heavily commented E2E
test case that serves as a practical template for testing a common `vk` use
case. The test follows the classic Arrange-Act-Assert pattern, a best practice
for structuring tests to be clear and understandable.6

**Scenario:** The test will verify the behavior of the `vk pr <url>` command
for a pull request that contains one unresolved comment thread. It will ensure
the command succeeds and that the rendered terminal output is correct.

```rust
// In tests/e2e.rs

use assert_cmd::Command;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use third_wheel::{ThirdWheel, MitmProxy};
use third_wheel::hyper::{Body, Request, Response, StatusCode};

// Helper functions `start_mock_server` and `load_fixture` are assumed
// to be defined as shown in Section 3.

// The test function must be an async function running on the tokio runtime
// to accommodate the asynchronous mock server.
#[tokio::test]
async fn test_pr_command_with_single_comment_renders_correctly() {
    //=========================================================================
    // 1. ARRANGE: Set up the entire test environment.
    //=========================================================================

    // Start the embedded third-wheel mock server. This gives us its address
    // and a handle to control its response behavior.
    let (mock_server_addr, handler) = start_mock_server().await;
    let mock_server_url = format!("http://{}", mock_server_addr);

    // Load the mock API response from a JSON fixture file. This ensures the
    // test uses a consistent and realistic data payload.
    let mock_response_body = load_fixture("pr_with_comments.json");

    // Configure the mock server's behaviour for this test.
    // The handler mutex is locked and a closure assigned for
    // any incoming request.
    *handler.lock().unwrap() = Box::new(move |_req: &Request<Body>| {
        Response::builder()
           .status(StatusCode::OK)
           .header("Content-Type", "application/json")
           .body(Body::from(mock_response_body.clone()))
           .unwrap()
    });

    //=========================================================================
    // 2. ACT: Execute the command-line tool.
    //=========================================================================

    // Create a command to run the `vk` binary.
    let mut cmd = Command::cargo_bin("vk").unwrap();

    // Configure the command's environment and arguments.
    cmd
        // Redirect vk to the mock API server. This step isolates the
        // test from the network.
       .env("GITHUB_GRAPHQL_URL", &mock_server_url)
        // Provide a dummy token, as the application may require one,
        // even if the mock server does not validate it.
       .env("GITHUB_TOKEN", "dummy_token")
        // Pass the command-line arguments to simulate the user's action.
       .args(&["pr", "https://github.com/leynos/vk/pull/1"]);

    //=========================================================================
    // 3. ASSERT: Verify the outcome of the execution.
    //=========================================================================

    // Execute the command and capture the full output (stdout, stderr, status).
    let output = cmd.output().expect("Failed to execute vk command");

    // First, assert that the process exited successfully.
    // A non-zero exit code would indicate a crash or an unexpected error.
    assert!(output.status.success(), "Command should exit with success code");

    // Convert the raw stdout bytes to a string for snapshot testing.
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is not valid UTF-8");

    // Assert that stderr is empty, as no errors or warnings are expected.
    assert_eq!(stderr, "", "stderr should be empty on success");

    // Finally, use insta to perform a snapshot assertion on the stdout.
    // This will compare the full, styled terminal output against the
    // stored snapshot in `tests/snapshots/e2e__....snap`.
    insta::assert_snapshot!(stdout);
}
```

This complete example demonstrates the synergy of the testing stack. `tokio`
and `third-wheel` create the controlled environment, `assert_cmd` executes the
application within that environment, and `insta` provides the final, robust
verification of the application's primary output. This structure forms a
powerful and reusable pattern for all other E2E tests in the suite.

## Advanced Techniques and Maintainability Patterns

A test suite is a living part of a software project that must be maintained and
scaled over time. This section covers advanced techniques for testing complex
scenarios and discusses best practices that ensure the test suite remains
clean, understandable, and effective as the `vk` tool evolves.

### Testing Configuration Sources with tempfile

The `vk` tool uses the `ortho_config` library, which provides a layered
configuration system. Settings can be sourced from command-line arguments,
environment variables, or configuration files. While testing arguments and
environment variables is straightforward with `assert_cmd`, testing file-based
configuration presents a challenge: how to do so without creating permanent
test files that clutter the project repository and create stateful dependencies
between test runs.

The `tempfile` crate provides an elegant solution by enabling the creation of
temporary files and directories that are automatically cleaned up when they go
out of scope.19

The following example demonstrates how to test that `vk` correctly reads a
repository setting from a temporary configuration file.

```rust
use assert_cmd::Command;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_configuration_is_read_from_temp_file() {
    // ARRANGE
    // 1. Create a temporary configuration file.
    let mut config_file = NamedTempFile::new().unwrap();
    // 2. Write the desired configuration into the file.
    writeln!(config_file, "repo = \"leynos/vk\"").unwrap();

    // 3. Set up the mock server (as in previous sections).
    let (addr, handler) = start_mock_server().await;
    let mock_response = load_fixture("pr_with_comments.json");
    *handler.lock().unwrap() = Box::new(move |_req| {
        Response::builder().body(Body::from(mock_response.clone())).unwrap()
    });

    // ACT
    let mut cmd = Command::cargo_bin("vk").unwrap();
    cmd
       .env("GITHUB_GRAPHQL_URL", format!("http://{}", addr))
       .env("GITHUB_TOKEN", "dummy_token")
        // 4. Point the command to the temporary config file.
        // Here we use an argument, but one could also use.current_dir()
        // if the app searches in the current directory.
       .arg("--config")
       .arg(config_file.path()) // Pass the path of the temporary file.
       .args(&["pr", "1"]);     // Use a PR number, relying on the repo from the file.

    // ASSERT
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    insta::assert_snapshot!(stdout);

    // 5. Cleanup is automatic. `config_file` is dropped here, and the
    // underlying file on disk is deleted.
}
```

This pattern allows for exhaustive testing of all file-based configuration
scenarios in a completely isolated and self-cleaning manner.

### Strategies for Testing Error Conditions

Thoroughly testing failure paths is just as critical as testing successful
ones. A robust application should fail gracefully and provide clear, actionable
feedback to the user.23 The E2E test suite should verify this behavior.

Here is a checklist of essential error conditions to test for `vk`:

- **Invalid User Input:**

  - **Scenario:** The user provides an invalid pull request reference (e.g.,
    `vk pr not-a-number`).

  - **Test Strategy:** Run the command with the invalid input. Use `assert_cmd`
    to assert `.failure()` and
    `.stderr(predicate::str::contains("Invalid reference"))`. No API mocking is
    needed as the input validation should fail early.

- **API-Level Errors:**

  - **Scenario:** The user requests a pull request that does not exist (e.g.,
    `vk pr 999999`). The GitHub API will return a valid JSON response
    containing an `errors` object.

  - **Test Strategy:** Configure `third-wheel` to serve a fixture containing a
    GraphQL error payload. Assert that `vk` exits with a failure code and that
    the error message from the API is printed to `stderr`.

- **Network Failures:**

  - **Scenario:** The GitHub API is down or there is a network connectivity
    issue.

  - **Test Strategy:** Configure `third-wheel` to return an HTTP
    `503 Service Unavailable` status code or to simply close the connection
    without responding. Assert that `vk` fails and prints a user-friendly
    network error message to `stderr` (e.g., "Error: Failed to connect to
    GitHub API").

- **Missing Configuration:**

  - **Scenario:** The user runs a command like `vk pr 123` without having
    specified a repository via a config file, environment variable (`VK_REPO`),
    or a full URL.

  - **Test Strategy:** Run the command with a cleared environment
    (`.env_clear()`) and no config file argument. Assert that the command fails
    and `stderr` contains a message like "Error: Repository not specified".

### Best Practices for a Scalable Test Suite

As a project grows, so does its test suite. Without deliberate care, tests can
become slow, brittle, and difficult to maintain. The following best practices
help ensure the long-term health of the test suite.

- **Descriptive Test Naming:** Test function names should clearly describe the
  scenario being tested. A name like
  `test_pr_command_fails_gracefully_on_api_error` is far more informative than
  `test_pr_error1`.30 This makes it easier to identify the purpose of a test
  and to diagnose failures.

- **Refactor with Helper Functions:** Repetitive setup and teardown logic
  should be extracted into helper functions. The `start_mock_server` function
  is a prime example. This practice, often referred to as making code DRY
  (Don't Repeat Yourself), keeps the body of the test functions focused on the
  specific Arrange-Act-Assert logic for that scenario, making them shorter and
  easier to read.21 These helpers can be placed in a shared

  `tests/helpers.rs` module.

- **Logical Test Organization:** As the number of tests increases, group them
  into separate files based on the functionality they cover. For example 16:

  - `tests/pr_commands.rs`: Tests related to the `vk pr` subcommand.

  - `tests/issue_commands.rs`: Tests for the `vk issue` subcommand.

  - tests/config.rs: Tests specifically for configuration logic.

    Cargo will automatically discover and run all tests in these files. This
    modular structure makes the test suite much easier to navigate.

- **The Library/Binary Crate Pattern:** For maximum testability and code reuse,
  consider structuring the project with a library crate (`src/lib.rs`) and a
  very thin binary crate (`src/main.rs`).15 The library would contain all the
  core application logic (API interaction, data processing, rendering logic),
  while the binary would only be responsible for parsing command-line arguments
  and calling the library functions. This pattern allows for the core logic to
  be tested with traditional unit and integration tests, complementing the
  black-box E2E suite and providing a more layered testing strategy. While a
  full refactoring is beyond the scope of this guide, it represents a mature
  evolutionary path for the project's architecture.

By adopting these advanced techniques and maintainability patterns, the `vk`
test suite can effectively scale with the application, providing a lasting
foundation of quality and confidence for future development.

## Works cited

 1. People following @[vee.cool](http://vee.cool) — Bluesky, accessed on July
    20, 2025, <https://web-cdn.bsky.app/profile/vee.cool/followers>

 1. Canop/termimad: A library to display rich (Markdown) snippets and texts in
    a rust terminal application - GitHub, accessed on July 20, 2025,
    <https://github.com/Canop/termimad>

 1. Termimad: use Markdown to display rich text in a terminal application -
    Rust Users Forum, accessed on July 20, 2025,
    <https://users.rust-lang.org/t/termimad-use-markdown-to-display-rich-text-in-a-terminal-application/29386>

 1. termimad - Rust - [Docs.rs](http://Docs.rs), accessed on July 20, 2025,
    <https://docs.rs/termimad>

 1. The Hitchhiker's Guide to E2E Testing | by Tally Barak - Medium, accessed
    on July 20, 2025,
    <https://tally-b.medium.com/the-hitchhikers-guide-to-e2e-testing-b2a9eebeeb27>

 1. How to Write Tests - The Rust Programming Language - Rust Documentation,
    accessed on July 20, 2025,
    <https://doc.rust-lang.org/book/ch11-01-writing-tests.html>

 1. termimad - [crates.io](http://crates.io): Rust Package Registry, accessed
    on July 20, 2025, <https://crates.io/crates/termimad/0.9.7>

 1. assert_cmd - Rust - [Docs.rs](http://Docs.rs), accessed on July 20, 2025,
    <https://docs.rs/assert_cmd>

 1. assert_cmd - [crates.io](http://crates.io): Rust Package Registry, accessed
    on July 20, 2025, <https://crates.io/crates/assert_cmd>

 1. assert-rs/assert_cmd - Command - GitHub, accessed on July 20, 2025,
    <https://github.com/assert-rs/assert_cmd>

 1. campbellC/third-wheel: A rust implementation of a man-in … - GitHub,
    accessed on July 20, 2025, <https://github.com/campbellC/third-wheel>

 1. Overview | Insta Snapshots, accessed on July 20, 2025,
    <https://insta.rs/docs/>

 1. insta - Rust - [Docs.rs](http://Docs.rs), accessed on July 20, 2025,
    <https://docs.rs/insta>

 1. Insta Snapshots, accessed on July 20, 2025, <https://insta.rs/>

 1. Testing - Command Line Applications in Rust, accessed on July 20, 2025,
    <https://rust-cli.github.io/book/tutorial/testing.html>

 1. Test Organization - The Rust Programming Language, accessed on July 20,
    2025, <https://doc.rust-lang.org/book/ch11-03-test-organization.html>

 1. insta - [crates.io](http://crates.io): Rust Package Registry, accessed on
    July 20, 2025, <https://crates.io/crates/insta>

 1. third-wheel - [crates.io](http://crates.io): Rust Package Registry,
    accessed on July 20, 2025, <https://crates.io/crates/third-wheel>

 1. tempfile - Rust - [Docs.rs](http://Docs.rs), accessed on July 20, 2025,
    <https://docs.rs/tempfile>

 1. Should unit tests really be put in the same file as the source? - Rust
    Users Forum, accessed on July 20, 2025,
    <https://users.rust-lang.org/t/should-unit-tests-really-be-put-in-the-same-file-as-the-source/62153>

 1. Skeleton And Principles For A Maintainable Test Suite | Luca Palmieri,
    accessed on July 20, 2025,
    <https://lpalmieri.com/posts/skeleton-and-principles-for-a-maintainable-test-suite/>

 1. Command in assert_cmd::cmd - Rust - [Docs.rs](http://Docs.rs), accessed on
    July 20, 2025,
    <https://docs.rs/assert_cmd/latest/assert_cmd/cmd/struct.Command.html>

 1. How I test Rust command-line apps with assert_cmd - alexwlchan, accessed on
    July 20, 2025,
    <https://alexwlchan.net/2025/testing-rust-cli-apps-with-assert-cmd/>

 1. assert_cmd for n00bs : r/rust - Reddit, accessed on July 20, 2025,
    <https://www.reddit.com/r/rust/comments/e2kfsr/assert_cmd_for_n00bs/>

 1. Snapshot Testing - Rust Project Primer, accessed on July 20, 2025,
    <https://www.rustprojectprimer.com/testing/snapshot.html>

 1. Snapshot testing - Advanced Rust testing - Rust Exercises, accessed on July
    20, 2025,
    <https://rust-exercises.com/advanced-testing/02_snapshots/00_intro.html>

 1. insta - Rust, accessed on July 20, 2025,
    <https://prisma.github.io/prisma-engines/doc/insta/index.html>

 1. tempfile - Rust - [Docs.rs](http://Docs.rs), accessed on July 20, 2025,
    <https://docs.rs/tempfile/latest/tempfile/>

 2. Complete Guide To Testing Code In Rust | Zero To Mastery, accessed on July
    20, 2025,
    <https://zerotomastery.io/blog/complete-guide-to-testing-code-in-rust/>

 3. Ultimate Guide to Testing and Debugging Rust Code | 2024 - Rapid
    Innovation, accessed on July 20, 2025,
    <https://www.rapidinnovation.io/post/testing-and-debugging-rust-code>
