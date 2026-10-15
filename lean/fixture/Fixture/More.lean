import Fixture.Basic

namespace Fixture

/-- Uses a lemma that depends on sorry. -/
theorem uses_hard (n : Nat) : double n = 2 * n := hard n

theorem uses_axiom : ∃ m, m = double 0 := choice_like 0

def unknown_twice : Nat := unknown + unknown

theorem classical (p : Prop) : p ∨ ¬p := Classical.em p

end Fixture
