use askama::Template;

#[derive(Template)]
#[template(path = "struct_channel_type.txt")]
pub(crate) struct ChannelTypeTemplate<'a> {
    pub(crate) name: &'a str,
}


#[derive(Template)]
#[template(path = "file_cargo.txt")]
pub(crate) struct CargoTemplate<'a> {
    pub(crate) name: &'a str,
}
#[derive(Template)]
#[template(path = "file_gitignore.txt")]
pub(crate) struct GitIgnoreTemplate {
}

#[derive(Template)]
#[template(path = "file_args.txt")]
pub(crate) struct ArgsTemplate {
}

#[derive(Template)]
#[template(path = "file_main.txt")]
pub(crate) struct MainTemplate {
}