"""Compile the two old adapter methods against deterministic, in-memory fixtures."""
import shutil
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
B = HERE.parents[3] / "analysis_official_streaming/lenovo_phase_impl_20261101/B_source/alvr"


def method(path: str, name: str) -> str:
    text = (B/path).read_text(encoding="utf-8")
    start = text.index(f"pub fn {name}(")
    depth = 0
    for offset in range(text.index("{", start), len(text)):
        depth += (text[offset] == "{") - (text[offset] == "}")
        if not depth:
            return text[start:offset+1]
    raise ValueError(name)


def main() -> None:
    folder = HERE / "results"
    folder.mkdir(exist_ok=True)
    source = r"""
use std::{time::{Instant,Duration},cell::{RefCell,RefMut}};
mod alvr_packets {pub mod vrx_phase {pub enum VrxPhaseMessage {Capability{},Feedback{id:u64},Offer}}}
use alvr_packets::vrx_phase::VrxPhaseMessage as Msg;
#[derive(PartialEq)] enum Platform {ThinkRealityVRX}
struct Slots(RefCell<[Option<Msg>;2]>);
impl Slots {fn lock(&self)->RefMut<'_,[Option<Msg>;2]> {self.0.borrow_mut()}}
struct Connection {vrx_phase_pending:Slots}
struct Client {platform:Platform,connection_context:Connection}
impl Client { SEND }
struct Phase;
impl Phase {fn tick(&mut self,_:Instant)->Option<(Duration,i64)>{Some((Duration::from_millis(11),-1_000_000))}}
struct Stats {vrx_phase:Option<Phase>,frame_interval:Duration,nominal_frame_interval:Duration,last_vsync_time:Instant}
impl Stats { WAIT }
fn main(){
 let c=Client{platform:Platform::ThinkRealityVRX,connection_context:Connection{vrx_phase_pending:Slots(RefCell::new([None,None]))}};
 for id in 1..=3 {c.send_vrx_phase(Msg::Feedback{id})}
 let slots=c.connection_context.vrx_phase_pending.lock();
 assert!(matches!(slots[1],Some(Msg::Feedback{id:3})));
 assert_eq!(slots.iter().flatten().count(),1);
 println!("old mailbox: 3 submitted, 1 retained, latest id=3");
 let period=Duration::from_millis(11);
 let mut stats=Stats{vrx_phase:Some(Phase),frame_interval:period,nominal_frame_interval:period,last_vsync_time:Instant::now()-Duration::from_micros(10500)};
 let wait=stats.duration_until_next_vsync();
 println!("old scheduling: corrected deadline already due; returned {:.3}ms",wait.as_secs_f64()*1000.);
 assert!(wait>Duration::from_millis(9));
}
"""
    # Keep the original method bodies; only their dependencies are substituted.
    fixture = folder/"old_adapter_fixture.rs"
    fixture.write_text(source.replace(" SEND ",method("client_core/src/lib.rs","send_vrx_phase"))
                       .replace(" WAIT ",method("server_core/src/statistics.rs","duration_until_next_vsync")))
    binary = folder/"old_adapter_fixture.exe"
    subprocess.run([shutil.which("rustc") or "rustc","--edition=2024","-Awarnings",str(fixture),
                    "-o",str(binary)],check=True)
    subprocess.run([str(binary)],check=True)


if __name__ == "__main__":
    main()
