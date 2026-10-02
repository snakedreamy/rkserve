pub mod plugin {
    pub mod v1 {
        tonic::include_proto!("rkserve.plugin.v1");
    }
}

pub const PROTOCOL_VERSION: u32 = 2;
