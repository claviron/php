use super::*;
use crate::{PhpScriptAdapter, PhpScriptService};

struct Script {
    adapter: PhpScriptAdapter,
    execution: claviron_runtime::ExecutorSpec,
}

pub(super) fn prepare(
    state: &AppState,
    runtime: Arc<PhpRuntime>,
) -> Result<Arc<dyn BackendHandler>> {
    let Some(service) = state.registry_ref().resolve::<PhpScriptService>()
    else {
        return Err(claviron_runtime::registry::Error::msg(
            "PhpScriptService is not registered",
        ));
    };
    let execution = service.execution();
    Ok(Arc::new(Script {
        adapter: PhpScriptAdapter::new(service, runtime),
        execution,
    }))
}

impl DispatchStep for Script {
    fn call(
        &self,
        request: Request<RouteBody>,
        _context: Option<claviron_pipeline::HostContext<'_>>,
    ) -> DispatchCall {
        let adapter = self.adapter.clone();
        DispatchCall::new(async move {
            match adapter.try_handle(request).await {
                Ok(ServiceResult::Handled(response)) => {
                    ServiceResult::Handled(response)
                }
                Ok(ServiceResult::Unhandled(request)) => {
                    // The native resolver returned before polling the body.
                    ServiceResult::Unhandled(request)
                }
                Err(error) => ServiceResult::Handled(gateway_error(
                    error.status_code(),
                    error,
                )),
            }
        })
    }
}

impl BackendHandler for Script {
    fn contract(&self) -> BackendContract {
        BackendContract {
            execution: self.execution.clone(),
            identity: BackendIdentity::Request,
            reply: BackendReply::Owner,
        }
    }
}
