use httpboot_protocol::KernelPublishResponse;

#[derive(Debug, Clone)]
pub struct KernelPublishInput {
    pub kernel_url: String,
    pub kernel_size: u64,
    pub kernel_sha256: Option<String>,
}

pub fn publish_kernel(input: KernelPublishInput) -> KernelPublishResponse {
    KernelPublishResponse {
        boot_id: uuid::Uuid::new_v4().to_string(),
        kernel_url: input.kernel_url,
        kernel_size: input.kernel_size,
        kernel_sha256: input.kernel_sha256,
    }
}
