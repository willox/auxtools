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


struct Disassembler<'a, I>
where
	I: Iterator<Item = &'a u32>,
{
	iter: I,
	current_offset: u32,
}

impl<'a, I> Disassembler<'a, I>
where
	I: Iterator<Item = &'a u32>,
{
	fn new(mut iter: I) -> Self {
		Self {
			iter,
			current_offset: 0
		}
	}

	fn next(&mut self) -> Option<&u32> {
		self.current_offset += 1;
		self.iter.next()
	}

	fn disassemble_instruction(&mut self) -> Result<(u32, u32, Instruction), DisassembleError>
	{
		let starting_offset = self.current_offset;
		let opcode = self.next().ok_or(Finished)?;
	
		// u32 -> repr(u32)
		let opcode: OpCode = unsafe { std::mem::transmute(*opcode) };
	
		let res = match opcode {
			OpCode::GetVar => Instruction::GetVar(self.disassemble_variable_operand()?),
			OpCode::PushVal => Instruction::PushVal(self.disassemble_pushval_operand()?),
			OpCode::DbgFile => Instruction::DbgFile(self.disassemble_string_operand()?),
			OpCode::DbgLine => Instruction::DbgLine(self.disassemble_u32_operand()?),
			_ => return Err(UnknownInstruction),
		};

		Ok((starting_offset, self.current_offset - 1, res))
	}

	fn disassemble_access_modifier_type(&mut self) -> Result<AccessModifier, DisassembleError>
	{
		let operand = self.disassemble_u32_operand()?;
		Ok(unsafe { std::mem::transmute(operand) })
	}
	
	fn disassemble_access_modifier_operands(&mut self, modifier: AccessModifier) -> Result<Variable, DisassembleError>
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
				Ok(Variable::Arg(self.disassemble_u32_operand()?))
			}
			AccessModifier::Local => {
				Ok(Variable::Local(self.disassemble_u32_operand()?))
			}
			AccessModifier::Global => {
				Ok(Variable::Global(self.disassemble_variable_name_operand()?))
			}
			// These modifiers have potentially recursive behaviour and are handled outside of this method
			AccessModifier::Field => {
				Err(UnknownAccessModifier)
			}
			AccessModifier::Initial => {
				Err(UnknownAccessModifier)
			}
			_ => Err(UnknownAccessModifier),
		}
	}
	
	fn disassemble_variable_field_chain(&mut self) -> Result<Variable, DisassembleError>
	{
		let obj = self.disassemble_access_modifier_type()?;
		let obj = self.disassemble_access_modifier_operands(obj)?;
	
		let mut fields = vec![];
	
		loop {
			// This is either a string-ref. AccessModifier::Field or AccessModifier::Initial
			let data = self.disassemble_u32_operand()?;
	
			if AccessModifier::Field as u32 == data {
				fields.push(self.disassemble_string_operand()?);
				continue;
			}
	
			if AccessModifier::Initial as u32 == data {
				fields.push(self.disassemble_string_operand()?);
				return Ok(Variable::InitialField(Box::new(obj), fields));
			}
	
			fields.push( unsafe {
				StringRef::from_id(raw_types::strings::StringId(data))
			});
	
			return Ok(Variable::Field(Box::new(obj), fields));
		}
	}
	
	fn disassemble_variable_operand(&mut self) -> Result<Variable, DisassembleError>
	{
		let modifier = self.disassemble_access_modifier_type()?;
	
		match modifier {
			AccessModifier::Field => {
				Ok(self.disassemble_variable_field_chain()?)
			},
			_ => self.disassemble_access_modifier_operands(modifier),
		}
	}
	
	fn disassemble_pushval_operand(&mut self) -> Result<Value, DisassembleError>
	{
		let tag = self.disassemble_u32_operand()? as u8;
		let tag = unsafe { std::mem::transmute(tag) };
	
		// Numbers store their data portion in the lower 16-bits of two operands
		if tag == raw_types::values::ValueTag::Number {
			let val1 = self.disassemble_u32_operand()?;
			let val2 = self.disassemble_u32_operand()?;
			return Ok(Value::from(f32::from_bits((val1 << 16) | val2)));
		}
	
		let data = self.disassemble_u32_operand()?;
	
		unsafe { Ok(Value::new(tag, raw_types::values::ValueData { id: data })) }
	}
	
	fn disassemble_u32_operand(&mut self) -> Result<u32, DisassembleError>
	{
		let operand = self.next().ok_or(UnexpectedEnd)?;
		Ok(*operand)
	}
	
	fn disassemble_variable_name_operand(&mut self) -> Result<StringRef, DisassembleError>
	{
		let operand = self.next().ok_or(UnexpectedEnd)?;
		let str = unsafe { StringRef::from_variable_id(raw_types::strings::VariableId(*operand)) };
	
		Ok(str)
	}
	
	fn disassemble_string_operand(&mut self) -> Result<StringRef, DisassembleError>
	{
		let operand = self.next().ok_or(UnexpectedEnd)?;
		let str = unsafe { StringRef::from_id(raw_types::strings::StringId(*operand)) };
	
		Ok(str)
	}
}

pub fn disassemble(proc: &Proc) -> Option<Vec<(u32, u32, Instruction)>> {
	let bytecode = unsafe {
		let (ptr, count) = proc.bytecode();
		std::slice::from_raw_parts(ptr, count)
	};

	let mut dism = Disassembler::new(bytecode.iter());

	let mut ret = vec![];
	loop {
		let offset = dism.current_offset;
		if let Ok(res) = dism.disassemble_instruction() {
			ret.push(res);
			continue;
		}
		break;
	}

	// TODO: Error

	Some(ret)
}
