mod db;
mod error;
mod messages;
mod sequence;

pub use db::MsgDb;
pub use error::{Error, Result};
pub use messages::Messages;


#[cfg(test)]
mod test{

    #[test]
    fn send_msg() {

    }

}


