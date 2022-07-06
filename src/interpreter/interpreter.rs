use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SlotMap};

use super::contexts::FullContext;
use super::error::RuntimeError;
use super::value::{value_ops, Value, ValueType};

use crate::compiler::compiler::{Code, Instruction};
use crate::interpreter::value::{Macro, Pattern};
use crate::sources::CodeArea;

new_key_type! {
    pub struct ValueKey;
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredValue {
    pub value: Value,
    pub def_area: CodeArea,
}

pub struct Globals {
    pub memory: SlotMap<ValueKey, StoredValue>,
    pub types: AHashMap<String, ValueType>,

    pub contexts: FullContext,
}
impl Globals {
    pub fn init(&mut self) {
        self.types.insert("int".into(), ValueType::Int);
        self.types.insert("float".into(), ValueType::Float);
        self.types.insert("string".into(), ValueType::String);
        self.types.insert("bool".into(), ValueType::Bool);
        self.types.insert("empty".into(), ValueType::Empty);
        self.types.insert("array".into(), ValueType::Array);
        self.types.insert("dictionary".into(), ValueType::Dict);
        self.types.insert("maybe".into(), ValueType::Maybe);
        self.types
            .insert("type_indicator".into(), ValueType::TypeIndicator);
        self.types.insert("pattern".into(), ValueType::Pattern);
        self.types.insert("group".into(), ValueType::Group);
        self.types
            .insert("trigger_function".into(), ValueType::TriggerFunc);
        self.types.insert("macro".into(), ValueType::Macro);
    }
}

pub fn execute(globals: &mut Globals, code: &Code, func: usize) -> Result<(), RuntimeError> {
    let mut stack: Vec<*mut StoredValue> = vec![];

    macro_rules! pop_clone {
        () => {
            unsafe { (*stack.pop().unwrap()).clone() }
        };
    }
    macro_rules! pop {
        (&) => {
            unsafe { &(*stack.pop().unwrap()) }
        };
        (&mut) => {
            unsafe { &mut (*stack.pop().unwrap()) }
        };
    }

    macro_rules! push {
        ($v:expr) => {{
            #[allow(unused_unsafe)]
            let key = unsafe { globals.memory.insert($v) };
            stack.push(&mut globals.memory[key]);
        }};
    }

    for context in globals.contexts.iter() {
        let mut i = 0;
        while i < code.instructions[func].0.len() {
            macro_rules! op_helper {
                (
                    $($instr:ident: $func:ident,)*
                ) => {
                    match &code.instructions[func].0[i] {
                        $(
                            Instruction::$instr => {
                                let area = code.get_bytecode_area(func, i);
                                let b = stack.pop().unwrap();
                                let a = stack.pop().unwrap();
                                let key = unsafe { globals.memory.insert(value_ops::$func(&*a, &*b, area)?) };
                                stack.push(&mut globals.memory[key]);
                            }
                        )*
                        _ => (),
                    }
                };
            }

            op_helper! {
                Plus: plus,
                Minus: minus,
                Mult: mult,
                Div: div,
                Mod: modulo,
                Pow: pow,
                Eq: eq,
                NotEq: not_eq,
                Greater: greater,
                GreaterEq: greater_eq,
                Lesser: lesser,
                LesserEq: lesser_eq,
            };

            match &code.instructions[func].0[i] {
                Instruction::LoadConst(id) => {
                    let area = code.get_bytecode_area(func, i);
                    let key = globals
                        .memory
                        .insert(code.constants.get(*id).clone().into_stored(area));
                    stack.push(&mut globals.memory[key]);
                }
                Instruction::Negate => {
                    let area = code.get_bytecode_area(func, i);
                    let a = stack.pop().unwrap();
                    push!(value_ops::unary_negate(&*a, area)?);
                }
                Instruction::Not => {
                    let area = code.get_bytecode_area(func, i);
                    let a = stack.pop().unwrap();
                    push!(value_ops::unary_not(&*a, area)?);
                }
                Instruction::LoadVar(id) => stack.push(&mut globals.memory[context.get_var(*id)]),
                Instruction::SetVar(id) => {
                    let top = pop_clone!();
                    let key = globals.memory.insert(top);
                    *context.vars[*id as usize].last_mut().unwrap() = Some(key)
                }
                Instruction::Print => {
                    let top = &unsafe { &*stack.pop().unwrap() }.value;
                    println!("{}", ansi_term::Color::Green.bold().paint(top.to_str()))
                }
                Instruction::LoadType(id) => {
                    let area = code.get_bytecode_area(func, i);
                    let name = code.names.get(*id);
                    match globals.types.get(name) {
                        Some(typ) => {
                            push!(Value::TypeIndicator(*typ).into_stored(area))
                        }
                        None => {
                            return Err(RuntimeError::UndefinedType {
                                name: name.clone(),
                                area,
                            })
                        }
                    }
                }
                Instruction::BuildArray(len) => {
                    let area = code.get_bytecode_area(func, i);
                    let mut elems = vec![];
                    for _ in 0..*len {
                        elems.push(pop_clone!());
                    }
                    elems.reverse();
                    push!(Value::Array(elems).into_stored(area));
                }
                Instruction::PushEmpty => {
                    let area = code.get_bytecode_area(func, i);
                    push!(Value::Empty.into_stored(area));
                }
                Instruction::PopTop => {
                    stack.pop();
                }
                Instruction::Jump(id) => {
                    i = *code.destinations.get(*id);
                    continue;
                }
                Instruction::JumpIfFalse(id) => unsafe {
                    if !value_ops::to_bool(&*stack.pop().unwrap())? {
                        i = *code.destinations.get(*id);
                        continue;
                    }
                },
                Instruction::ToIter => todo!(),
                Instruction::IterNext(_) => todo!(),
                Instruction::BuildDict(id) => {
                    let area = code.get_bytecode_area(func, i);
                    let keys = code.name_sets.get(*id);
                    let map = keys
                        .iter()
                        .cloned()
                        .zip((0..keys.len()).map(|_| pop_clone!()))
                        .collect();
                    push!(Value::Dict(map).into_stored(area));
                }
                Instruction::Return => todo!(),
                Instruction::Continue => todo!(),
                Instruction::Break => todo!(),
                Instruction::MakeMacro(id) => {
                    let area = code.get_bytecode_area(func, i);
                    let (func_id, arg_info) = code.macro_build_info.get(*id);
                    let ret_type = Box::new(pop_clone!());
                    let mut args = vec![];
                    for (name, typ, def) in arg_info {
                        let def = if *def { Some(pop_clone!()) } else { None };
                        let typ = if *typ { Some(pop_clone!()) } else { None };
                        args.push((name.clone(), typ, def));
                    }
                    args.reverse();
                    push!(Value::Macro(Macro {
                        func_id: *func_id,
                        args,
                        ret_type
                    })
                    .into_stored(area));
                }
                Instruction::PushAnyPattern => {
                    let area = code.get_bytecode_area(func, i);
                    push!(Value::Pattern(Pattern::Any).into_stored(area));
                }
                Instruction::MakeMacroPattern(_) => todo!(),
                Instruction::Index => todo!(),
                Instruction::Call(id) => {
                    let area = code.get_bytecode_area(func, i);
                    let base = pop!(&);
                    match &base.value {
                        Value::Macro(m) => {
                            let mut params = vec![];
                            let mut named_params = AHashMap::new();
                            let param_list = code.name_sets.get(*id);
                            for i in param_list {
                                if i.is_empty() {
                                    params.push(pop_clone!());
                                } else {
                                    named_params.insert(i.clone(), pop_clone!());
                                }
                            }
                            todo!()
                        }
                        _ => {
                            return Err(RuntimeError::CannotCall {
                                base: base.clone(),
                                area,
                            })
                        }
                    }
                }
                Instruction::TriggerFuncCall => todo!(),
                Instruction::SaveContexts => todo!(),
                Instruction::ReviseContexts => todo!(),
                Instruction::MergeContexts => {}
                Instruction::PushNone => todo!(),
                Instruction::WrapMaybe => todo!(),
                Instruction::PushContextGroup => todo!(),
                Instruction::PopContextGroup => todo!(),
                Instruction::PushTriggerFnValue => todo!(),
                Instruction::TypeDef(_) => todo!(),
                Instruction::Impl(_) => todo!(),
                Instruction::Instance(_) => todo!(),

                Instruction::Plus
                | Instruction::Minus
                | Instruction::Mult
                | Instruction::Div
                | Instruction::Mod
                | Instruction::Pow
                | Instruction::Eq
                | Instruction::NotEq
                | Instruction::Greater
                | Instruction::GreaterEq
                | Instruction::Lesser
                | Instruction::LesserEq => (),

                Instruction::EnterScope => {}
                Instruction::ExitScope => {}
            }

            i += 1;
        }
    }

    unsafe {
        println!(
            "stack: {}",
            stack
                .iter()
                .map(|s| format!("{:?}", (**s).value))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}