use crate::model::{BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError};

/// A provider for installations without API credentials: harness runs work
/// (they bring their own credentials), native runs fail with a clear message.
pub struct UnconfiguredProvider;

impl ModelProvider for UnconfiguredProvider {
    fn complete<'a>(
        &'a self,
        _: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        Box::pin(async {
            Err(ProviderError::Config(
                "no model provider configured: native runs need ANTHROPIC_API_KEY (harness runs \
                 don't)"
                    .into(),
            ))
        })
    }
}
