fn main() {
    // A panic is an internal error: exit 1, never the panic or abort status.
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        eprintln!("error: internal error: {message}");
        std::process::exit(1);
    }));
    std::process::exit(allowit_cli::run_os(std::env::args_os().skip(1).collect()));
}
