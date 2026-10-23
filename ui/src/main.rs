use probe_rs::probe::list::Lister;
use probe_rs::rtt::Rtt;
use probe_rs::Permissions;

fn main() {
    // First obtain a probe-rs session (see probe-rs documentation for details)
    let lister = Lister::new();

    let probes = lister.list_all();
    println!("{:?}", probes);

    let probe = probes[0].open().unwrap();
    let mut session = probe.attach("somechip", Permissions::default()).unwrap();
    // Select a core.
    let mut core = session.core(0).unwrap();

    // Attach to RTT
    let mut rtt = Rtt::attach(&mut core).unwrap();

    // Read from a channel
    if let Some(input) = rtt.up_channel(1) {
        let mut buf = [0u8; 1024];
        let count = input.read(&mut core, &mut buf[..]).unwrap();

        println!("Read data: {:?}", &buf[..count]);
    }
}
