use ahash::AHashMap;
use ahash::AHashSet;
use paste::paste;
use slotmap::new_key_type;
use slotmap::SlotMap;

use super::context::FullContext;
use super::context::SkipMode::*;
use super::error::RuntimeError;
use super::instructions;
use super::types::BuiltinFunction;
use super::types::CustomType;
use super::value::StoredValue;
use super::value::ValueType;

use crate::compilation::code::*;
use crate::compilation::compiler::CompilerGlobals;
use crate::leveldata::gd_types::ArbitraryId;
use crate::leveldata::object_data::GdObj;
use crate::vm::context::ReturnType;
use crate::vm::instructions::InstrData;

new_key_type! {
    pub struct ValueKey;
    pub struct TypeKey;
    pub struct BuiltinKey;
}

pub enum TypeMember {
    Builtin(BuiltinKey),
    Custom(ValueKey),
}

pub struct Globals {
    pub memory: SlotMap<ValueKey, StoredValue>,

    pub undefined_captured: AHashSet<VarID>,
    pub arbitrary_ids: [ArbitraryId; 4],

    pub objects: Vec<GdObj>,
    pub triggers: Vec<GdObj>,
    pub types: SlotMap<TypeKey, CustomType>,

    //pub type_keys: AHashMap<String, TypeKey>,
    pub type_members: AHashMap<ValueType, AHashMap<String, TypeMember>>,
    pub builtins: SlotMap<BuiltinKey, BuiltinFunction>,

    pub builtins_by_name: AHashMap<String, BuiltinKey>,
}

impl Globals {
    pub fn new() -> Self {
        let mut g = Self {
            memory: SlotMap::default(),

            undefined_captured: AHashSet::new(),
            arbitrary_ids: [0; 4],

            objects: Vec::new(),
            triggers: Vec::new(),
            types: SlotMap::default(),

            builtins: SlotMap::default(),

            type_members: AHashMap::default(),
            builtins_by_name: AHashMap::default(),
        };
        g.init_types();
        g
    }

    pub fn set_types(&mut self, types: SlotMap<TypeKey, CustomType>) {
        self.types = types;
    }
    pub fn key_deep_clone(&mut self, k: ValueKey) -> ValueKey {
        let val = self.memory[k].clone();
        let val = val.deep_clone(self);
        self.memory.insert(val)
    }
}

pub fn run_func(
    globals: &mut Globals,
    code: &Code,
    fn_index: usize,
    contexts: &mut FullContext,
    comp_globals: &CompilerGlobals,
) -> Result<(), RuntimeError> {
    let instructions = &code.funcs[fn_index].instructions;

    // set all context positions to 0
    for context in contexts.iter(IncludeReturns) {
        context.inner().pos = 0;
        assert!(context.inner().returned.is_none());
    }
    // run a function for each instruction
    macro_rules! instr_funcs {
        (
            ($contexts:ident, $instr:ident, $data:ident, $globals:ident)
            $($name:ident $(($arg:ident))?)+
        ) => {
            paste! {
                match $instr {
                    $(

                        Instruction::$name$(($arg))? => instructions::[<run_ $name:snake>]($globals, &$data, $contexts $(, *$arg)?)?,

                    )+
                }
            }
        };
    }

    'instruction_loop: loop {
        if instructions.is_empty() {
            break;
        }

        let mut finished = true;
        for context in contexts.iter(SkipReturns) {
            finished = false;

            let pos = context.inner().pos;
            let instr = &instructions[pos as usize].0;
            let data = InstrData {
                comp_globals,
                code,
                span: instructions[pos as usize].1,
            };

            instr_funcs! (
                (context, instr, data, globals)
                LoadConst(a)
                CallOp(a)
                LoadVar(a)
                SetVar(a)
                CreateVar(a)
                BuildArray(a)
                BuildDict(a)
                Jump(a)
                JumpIfFalse(a)
                UnwrapOrJump(a)
                PopTop
                PushEmpty
                WrapMaybe
                PushNone
                TriggerFuncCall
                PushTriggerFn
                Print
                ToIter
                IterNext(a)
                Impl
                PushAnyPattern
                BuildMacro(a)
                Call(a)
                CallBuiltin(a)
                Index
                Member(a)
                Associated(a)
                TypeOf
                Return
                YeetContext
                EnterArrowStatement(a)
                EnterTriggerFunction(a)

                BuildObject(a)
                BuildTrigger(a)
                AddObject

                BuildInstance
                PushBuiltins

                Import(a)
            );

            for context in context.iter(SkipReturns) {
                context.inner().pos += 1;
                if context.inner().pos >= instructions.len() as isize {
                    context.inner().returned = Some(ReturnType::Implicit);
                }
            }
        }

        if finished {
            break 'instruction_loop;
        }
    }
    if contexts
        .iter(IncludeReturns)
        .any(|c| matches!(c.inner().returned, Some(ReturnType::Explicit(_))))
    {
        contexts.yeet_implicit();
    }
    contexts.clean_yeeted();
    Ok(())
}