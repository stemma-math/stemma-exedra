/-
The inspection of a Stemma library.

Stemma's workers run it after building a commit of a group's library, with
the library's own toolchain and dependencies, from the library's folder:

    lake env lean --run Inspect.lean Library.Basic Library.Other …

It imports the modules named, and prints on standard output, as JSON, a
report on each declaration of the library (not of its dependencies): its
name, module, kind, pretty-printed type, docstring and place in its file;
whether it depends on `sorry`, and for a definition, whether its data (not
only its proofs) does; the axioms it depends on; and the declarations of
the library it uses. Declarations the compiler makes (recursors, matchers,
auxiliary lemmas…) are left out, and what they use is counted as used by
the declaration they belong to.

It uses only Lean itself, so that it runs with whatever version of Lean
the library uses.
-/
import Lean

open Lean Meta

namespace StemmaInspect

/-- Whether a name is one the compiler makes, rather than people. -/
def isMachineMade (env : Environment) (name : Name) : Bool :=
  name.isInternalDetail || isPrivateName name
    || isAuxRecursor env name || isNoConfusion env name
    || isMatcherCore env name
    || (match name with
        | .str _ s => s == "inj" || s == "injEq" || s == "noConfusionType" || s == "sizeOf_spec"
            || s == "eq_def" || s == "ctorIdx"
            || (s.startsWith "eq_" && (s.drop 3).all Char.isDigit)
            || (s.startsWith "proof_" && (s.drop 6).all Char.isDigit)
        | _ => false)
    || (match env.find? name with
        | some (.recInfo _) => true
        | _ => false)

/-- What kind of declaration a constant is, as people call it. -/
def kindOf (env : Environment) (info : ConstantInfo) : String :=
  match info with
  | .thmInfo _ => "theorem"
  | .defnInfo _ => if isInstanceCore env info.name then "instance" else "definition"
  | .opaqueInfo _ => if isInstanceCore env info.name then "instance" else "opaque"
  | .axiomInfo _ => "axiom"
  | .inductInfo _ =>
    if isClass env info.name then "class"
    else if isStructure env info.name then "structure"
    else "inductive"
  | .ctorInfo _ => "constructor"
  | .recInfo _ => "recursor"
  | .quotInfo _ => "quotient"

/-- The value of a constant, if it has one that holds data. -/
def valueOf? : ConstantInfo → Option Expr
  | .defnInfo d => some d.value
  | .opaqueInfo d => some d.value
  | _ => none

/-- Whether `sorry` stands for data somewhere in `e`: at a type that is not
a proposition. A `sorry` whose universe level is zero stands for a proof. -/
def hasDataSorry (e : Expr) : Bool :=
  (e.find? fun sub => match sub with
    | .const ``sorryAx [u] => u.normalize != Level.zero
    | _ => false).isSome

structure State where
  /-- Whether each declaration of the library has data depending on `sorry`. -/
  dataSorry : Std.HashMap Name Bool := {}
  /-- The library's declarations that each constant of it uses, through the
  ones the compiler made. -/
  uses : Std.HashMap Name NameSet := {}

abbrev M := StateT State MetaM

/-- Whether the data of the library's declaration `name` depends on `sorry`,
itself or through the library's definitions it uses. Dependencies outside
the library are taken to have none. -/
partial def dataSorry (inLibrary : Name → Bool) (name : Name) : M Bool := do
  if let some known := (← get).dataSorry[name]? then return known
  -- Assumed false while it is looked at, so that recursion ends.
  modify fun s => { s with dataSorry := s.dataSorry.insert name false }
  let env ← getEnv
  let result ← match env.find? name >>= valueOf? with
    | none => pure false
    | some value =>
      if hasDataSorry value then pure true
      else
        value.getUsedConstants.anyM fun c =>
          if inLibrary c && c != name then dataSorry inLibrary c else pure false
  modify fun s => { s with dataSorry := s.dataSorry.insert name result }
  return result

/-- The declarations of the library, other than `name`, that `name` uses in
its type or value, seeing through the ones the compiler made. -/
partial def usesOf (inLibrary : Name → Bool) (name : Name) : M NameSet := do
  if let some known := (← get).uses[name]? then return known
  modify fun s => { s with uses := s.uses.insert name {} }
  let env ← getEnv
  let some info := env.find? name | return {}
  let mut found : NameSet := {}
  for c in info.getUsedConstantsAsSet do
    if c == name || !inLibrary c then continue
    if isMachineMade env c then
      for d in ← usesOf inLibrary c do
        if d != name then found := found.insert d
    else
      found := found.insert c
  modify fun s => { s with uses := s.uses.insert name found }
  return found

def sortedNames (names : NameSet) : Array String :=
  (names.toArray.map toString).qsort (· < ·)

def rangeJson (range : DeclarationRange) : Json :=
  Json.mkObj [
    ("start_line", range.pos.line),
    ("start_column", range.charUtf16),
    ("end_line", range.endPos.line),
    ("end_column", range.endCharUtf16)]

/-- The report on one declaration. -/
def inspect (inLibrary : Name → Bool) (module : Name) (info : ConstantInfo) : M Json := do
  let env ← getEnv
  let name := info.name
  let type ← try
      pure ((← ppExpr info.type).pretty 100)
    catch _ =>
      pure (toString info.type)
  let doc ← findDocString? env name
  let range ← findDeclarationRanges? name
  let axioms ← collectAxioms name
  let isData := (valueOf? info).isSome && !info.isTheorem
  let dataSorry ← if isData then dataSorry inLibrary name else pure false
  let uses ← usesOf inLibrary name
  return Json.mkObj [
    ("name", toString name),
    ("module", toString module),
    ("kind", kindOf env info),
    ("type", type),
    ("doc", match doc with | some d => Json.str d | none => Json.null),
    ("range", match range with | some r => rangeJson r.range | none => Json.null),
    ("sorry", axioms.contains ``sorryAx),
    ("data_sorry", dataSorry),
    ("axioms", toJson ((axioms.map toString).qsort (· < ·))),
    ("uses", toJson (sortedNames uses))]

/-- The report on the library made of `modules`, in the environment that
imports them. -/
def report (modules : Array Name) : MetaM Json := do
  let env ← getEnv
  let moduleNames := env.header.moduleNames
  let moduleOf (name : Name) : Option Name :=
    (env.getModuleIdxFor? name).map fun idx => moduleNames[idx.toNat]!
  let inLibrary (name : Name) : Bool :=
    match moduleOf name with
    | some m => modules.contains m
    | none => false
  let mut found : Array (Name × Name × ConstantInfo) := #[]
  for (name, info) in env.constants.map₁.toList do
    let some module := moduleOf name | continue
    if !modules.contains module || isMachineMade env name then continue
    found := found.push (module, name, info)
  found := found.qsort fun (a b : Name × Name × ConstantInfo) =>
    a.1.toString < b.1.toString || (a.1 == b.1 && a.2.1.toString < b.2.1.toString)
  let (declarations, _) ← (found.mapM fun (entry : Name × Name × ConstantInfo) =>
    inspect inLibrary entry.1 entry.2.2).run {}
  return Json.mkObj [
    ("lean", Lean.versionString),
    ("modules", toJson (modules.map toString)),
    ("declarations", Json.arr declarations)]

end StemmaInspect

unsafe def main (args : List String) : IO UInt32 := do
  if args.isEmpty then
    IO.eprintln "usage: lake env lean --run Inspect.lean Module …"
    return 2
  let modules := args.toArray.map String.toName
  initSearchPath (← findSysroot)
  enableInitializersExecution
  let env ← importModules (modules.map fun m => { module := m }) {} (loadExts := true)
  let context : Core.Context := {
    fileName := "<inspection>"
    fileMap := default
    maxHeartbeats := 0 }
  let (json, _) ← (StemmaInspect.report modules).run' {} {} |>.toIO context { env }
  IO.println json.compress
  return 0
