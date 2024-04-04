

//  will support installing the app into a PATH where it can be used either by user or system.


use log::*;

#[derive(Clone)]
pub struct LocalCLIBuilder {
     pub(crate) path: String

}

impl LocalCLIBuilder {

    pub fn new(path: String) -> Self {
        LocalCLIBuilder {
            path,
        }
    }

    pub fn build(&self) {

        info!("path: {}", self.path);
    }

}
