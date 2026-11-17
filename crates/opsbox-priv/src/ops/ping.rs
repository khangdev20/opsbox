use opsbox_proto::PrivReply;

pub(super) fn run() -> PrivReply {
    PrivReply::Pong {
        version: crate::VERSION.to_owned(),
    }
}
