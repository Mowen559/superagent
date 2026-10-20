pub mod env_bootstrapper;
pub mod sandbox_runtime;
pub mod local_sandbox;
pub mod docker_sandbox;
pub mod spider_assertion;
pub mod spider_runner;
pub mod reflexion_engine;

pub use docker_sandbox::DockerSandbox;
pub use local_sandbox::LocalSandbox;
pub use sandbox_runtime::{ExecutionResult, SandboxRuntime};
pub use spider_assertion::{evaluate_spider_output, AssertionVerdict, SpiderAssertionSpec};
pub use spider_runner::{SpiderRunConfig, SpiderSandboxRunner};
pub use reflexion_engine::{ReflexionFeedback, ReflexionOutcome, SpiderReflexionSession};
pub use env_bootstrapper::{EnvBootstrapper, DetectedRuntime, DependencyReport, RuntimeKind};
