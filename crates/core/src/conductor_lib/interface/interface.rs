use crate::conductor_lib::api::ExternalConductorApi;
use async_trait::async_trait;

#[async_trait]
pub trait Interface {
    async fn spawn(self, api: ExternalConductorApi);
}
