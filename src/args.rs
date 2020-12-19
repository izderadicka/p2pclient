use structopt::StructOpt;

#[derive(StructOpt, Debug)]
pub struct Args {
    #[structopt(name = "PEER_ADDR")]
    pub peers: Vec<String>,

    #[structopt(long, short)]
    pub no_input: bool,
}

impl Args {
    pub fn from_args() -> Self {
        <Self as StructOpt>::from_args()
    }
}
