pub mod instructions;
pub mod opcodes;

pub use instructions::{Instruction, Variable};

use crate::raw_types;
use crate::Proc;
use crate::StringRef;
use crate::Value;
use opcodes::{AccessModifier, OpCode};

enum DisassembleError {
	UnexpectedEnd,
	UnknownInstruction,
	UnknownOperand,
	UnknownAccessModifier,
	Finished, // bad
}

use DisassembleError::*;

fn disassemble_access_modifier_type<'a, I>(
	mut iter: I,
) -> Result<AccessModifier, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let operand = disassemble_u32_operand(&mut iter)?;
	Ok(unsafe { std::mem::transmute(operand) })
}

fn disassemble_access_modifier_operands<'a, I>(mut iter: I, modifier: AccessModifier) -> Result<Variable, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	match modifier {
		AccessModifier::Null => {
			Ok(Variable::Null)
		},
		AccessModifier::World => {
			Ok(Variable::World)
		},
		AccessModifier::Src => {
			Ok(Variable::Src)
		}
		AccessModifier::Dot => {
			Ok(Variable::Dot)
		},
		AccessModifier::Cache => {
			Ok(Variable::Cache)
		},
		AccessModifier::Arg => {
			Ok(Variable::Arg(disassemble_u32_operand(&mut iter)?))
		}
		AccessModifier::Local => {
			Ok(Variable::Local(disassemble_u32_operand(&mut iter)?))
		}
		AccessModifier::Global => {
			Ok(Variable::Global(disassemble_variable_name_operand(&mut iter)?))
		}
		AccessModifier::Field => {
			Err(UnknownAccessModifier)
		}
		_ => Err(UnknownAccessModifier),
	}
}

fn disassemble_variable_field_chain<'a, I>(mut iter: I) -> Result<Variable, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let obj = disassemble_access_modifier_type(&mut iter)?;
	let obj = disassemble_access_modifier_operands(&mut iter, obj)?;

	let mut fields = vec![];

	loop {
		// This is either a string-ref or AccessModifier::Field
		let data = disassemble_u32_operand(&mut iter)?;

		if AccessModifier::Field as u32 == data {
			fields.push(disassemble_string_operand(&mut iter)?);
			continue;
		}

		fields.push( unsafe {
			StringRef::from_id(raw_types::strings::StringId(data))
		});

		break;
	}

	Ok(Variable::Field(Box::new(obj), fields))
}

fn disassemble_variable_operand<'a, I>(mut iter: I) -> Result<Variable, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let modifier = disassemble_access_modifier_type(&mut iter)?;

	match modifier {
		AccessModifier::Field => {
			Ok(disassemble_variable_field_chain(&mut iter)?)
		},
		_ => disassemble_access_modifier_operands(&mut iter, modifier),
	}
}

fn disassemble_pushval_operand<'a, I>(mut iter: I) -> Result<Value, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let tag = disassemble_u32_operand(&mut iter)? as u8;
	let tag = unsafe { std::mem::transmute(tag) };

	// Numbers store their data portion in the lower 16-bits of two operands
	if tag == raw_types::values::ValueTag::Number {
		let val1 = disassemble_u32_operand(&mut iter)?;
		let val2 = disassemble_u32_operand(&mut iter)?;
		return Ok(Value::from(f32::from_bits((val1 << 16) | val2)));
	}

	let data = disassemble_u32_operand(iter)?;

	unsafe { Ok(Value::new(tag, raw_types::values::ValueData { id: data })) }
}

fn disassemble_u32_operand<'a, I>(mut iter: I) -> Result<u32, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let operand = iter.next().ok_or(UnexpectedEnd)?;
	Ok(*operand)
}

fn disassemble_variable_name_operand<'a, I>(mut iter: I) -> Result<StringRef, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let operand = iter.next().ok_or(UnexpectedEnd)?;
	let str = unsafe { StringRef::from_variable_id(raw_types::strings::VariableId(*operand)) };

	Ok(str)
}

fn disassemble_string_operand<'a, I>(mut iter: I) -> Result<StringRef, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let operand = iter.next().ok_or(UnexpectedEnd)?;
	let str = unsafe { StringRef::from_id(raw_types::strings::StringId(*operand)) };

	Ok(str)
}

fn disassemble_instruction<'a, I>(mut iter: I) -> Result<Instruction, DisassembleError>
where
	I: Iterator<Item = &'a u32>,
{
	let opcode = iter.next().ok_or(Finished)?;

	// u32 -> repr(u32)
	let opcode: OpCode = unsafe { std::mem::transmute(*opcode) };

	match opcode {
		OpCode::GetVar => Ok(Instruction::GetVar(disassemble_variable_operand(&mut iter)?)),
		OpCode::PushVal => Ok(Instruction::PushVal(disassemble_pushval_operand(&mut iter)?)),
		OpCode::DbgFile => Ok(Instruction::DbgFile(disassemble_string_operand(&mut iter)?)),
		OpCode::DbgLine => Ok(Instruction::DbgLine(disassemble_u32_operand(&mut iter)?)),
		_ => Err(UnknownInstruction),
	}
}

pub fn disassemble(proc: &Proc) -> Option<Vec<Instruction>> {
	let bytecode = unsafe {
		let (ptr, count) = proc.bytecode();
		std::slice::from_raw_parts(ptr, count)
	};

	let mut ret = vec![];
	let mut it = bytecode.iter();
	while let Ok(ins) = disassemble_instruction(&mut it) {
		ret.push(ins);
	}

	// TODO: Error

	Some(ret)
}
