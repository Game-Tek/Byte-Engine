//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

pub struct Orchestrator {
}

unsafe impl Send for Orchestrator {}

pub type OrchestratorHandle = std::rc::Rc<std::cell::RefCell<Orchestrator>>;

impl Orchestrator {
	pub fn new() -> Orchestrator {
		Orchestrator {
		}
	}

	pub fn new_handle() -> OrchestratorHandle {
		std::rc::Rc::new(std::cell::RefCell::new(Orchestrator::new()))
	}
}

trait Parameter where Self: Sized {
	fn call<F: FnOnce(Self)>(orchestrator: &Orchestrator, closure: F);
}

pub trait TaskFunction<'a, PS> {
	fn call(self, orchestrator: &Orchestrator);
}

impl <'a, F, P0> TaskFunction<'a, (P0,)> for F where
	P0: Parameter,
	F: Fn(P0)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { (self)(p0) });
	}
}

impl <'a, F, P0, P1> TaskFunction<'a, (P0, P1)> for F where
	P0: Parameter,
	P1: Parameter,
	F: Fn(P0, P1)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { P1::call(orchestrator, move |p1| { (self)(p0, p1) }); });
	}
}

impl <'a, F, P0, P1, P2> TaskFunction<'a, (P0, P1, P2)> for F where
	P0: Parameter,
	P1: Parameter,
	P2: Parameter,
	F: Fn(P0, P1, P2)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { P1::call(orchestrator, move |p1| { P2::call(orchestrator, move |p2| { (self)(p0, p1, p2) }); }); });
	}
}

pub struct OrchestratorReference {
	pub(crate) handle: OrchestratorHandle,
	#[allow(dead_code)]
	pub(crate) internal_id: u32,
}

impl <'a> OrchestratorReference {
	pub fn get_handle(&self) -> OrchestratorHandle {
		self.handle.clone()
	}
}