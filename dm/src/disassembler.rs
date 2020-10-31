pub mod instructions;
pub mod opcodes;

use std::iter::Peekable;

pub use instructions::*;

use crate::raw_types;
use crate::Proc;
use crate::StringRef;
use crate::Value;
use opcodes::{AccessModifier, OpCode};

#[derive(Debug)]
pub enum DisassembleError {
	UnexpectedEnd,
	UnknownOp(OpCode),
	UnknownAccessModifier,
	InvalidProcId,
	Finished, // bad
}

use DisassembleError::*;

struct Disassembler<'a, I>
where
	I: Iterator<Item = &'a u32>,
{
	iter: Peekable<I>,
	current_offset: u32,
}

impl<'a, I> Disassembler<'a, I>
where
	I: Iterator<Item = &'a u32>,
{
	fn new(iter: I) -> Self {
		Self {
			iter: iter.peekable(),
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
			OpCode::IsNull => Instruction::IsNull,
			OpCode::IsNum => Instruction::IsNum,
			OpCode::IsText => Instruction::IsText,
			OpCode::IsPath => Instruction::IsPath,
			OpCode::IsSubPath => Instruction::IsSubPath,
			OpCode::IsType => Instruction::IsType,
			OpCode::IsList => Instruction::IsList,
			OpCode::ListGet => Instruction::ListGet,
			OpCode::ListSet => Instruction::ListSet,
			OpCode::Teq => Instruction::Teq,
			OpCode::Tne => Instruction::Tne,
			OpCode::Tl => Instruction::Tl,
			OpCode::Tg => Instruction::Tg,
			OpCode::Tle => Instruction::Tle,
			OpCode::Tge => Instruction::Tge,
			OpCode::GetFlag => Instruction::GetFlag,
			OpCode::Not => Instruction::Not,
			OpCode::Text2Path => Instruction::Text2Path,
			OpCode::NewArgList => Instruction::NewArgList,
			OpCode::ReplaceText => Instruction::ReplaceText,
			OpCode::Shell => Instruction::Shell,
			OpCode::FExists => Instruction::FExists,
			OpCode::File2Text => Instruction::File2Text,
			OpCode::Text2File => Instruction::Text2File,
			OpCode::FCopy => Instruction::FCopy,
			OpCode::FDel => Instruction::FDel,
			OpCode::SplitText => Instruction::SplitText,
			OpCode::Length => Instruction::Length,
			OpCode::Time2Text => Instruction::Time2Text,
			OpCode::Md5 => Instruction::Md5,
			OpCode::Sleep => Instruction::Sleep,
			OpCode::Min => Instruction::Min(self.disassemble_param_count_operand()?),
			OpCode::Max => Instruction::Max(self.disassemble_param_count_operand()?),
			OpCode::Inc => Instruction::Inc(self.disassemble_variable_operand()?),
			OpCode::TypesOf => Instruction::TypesOf(self.disassemble_param_count_operand()?),
			OpCode::Switch => Instruction::Switch(self.disassemble_switch_operand()?),
			OpCode::Jmp => Instruction::Jmp(self.disassemble_loc_operand()?),
			OpCode::Jmp2 => Instruction::Jmp2(self.disassemble_loc_operand()?),
			OpCode::AugAdd => Instruction::AugAdd(self.disassemble_variable_operand()?),
			OpCode::IterLoad => Instruction::IterLoad(self.disassemble_u32_operand()?, self.disassemble_u32_operand()?),
			OpCode::IterNext => Instruction::IterNext,
			OpCode::Format => Instruction::Format(self.disassemble_string_operand()?, self.disassemble_param_count_operand()?),
			OpCode::JsonEncode => Instruction::JsonEncode,
			OpCode::JsonDecode => Instruction::JsonDecode,
			OpCode::JmpOr => Instruction::JmpOr(self.disassemble_loc_operand()?),
			OpCode::NewAssocList => Instruction::NewAssocList(self.disassemble_param_count_operand()?),
			OpCode::Crash => Instruction::Crash,
			OpCode::NewList => Instruction::NewList(self.disassemble_u32_operand()?),
			OpCode::Try => Instruction::Try(self.disassemble_loc_operand()?),
			OpCode::Catch => Instruction::Catch(self.disassemble_loc_operand()?),
			OpCode::CallName => Instruction::CallName(self.disassemble_param_count_operand()?),
			OpCode::End => Instruction::End(),
			OpCode::Rand => Instruction::Rand,
			OpCode::RandRange => Instruction::RandRange,
			OpCode::New => Instruction::New(self.disassemble_param_count_operand()?),
			OpCode::Pop => Instruction::Pop,
			OpCode::Stat => Instruction::Stat,
			OpCode::Output => Instruction::Output,
			OpCode::CallNoReturn => Instruction::CallNoReturn(self.disassemble_variable_operand()?, self.disassemble_u32_operand()?),
			OpCode::Call => Instruction::Call(self.disassemble_variable_operand()?, self.disassemble_u32_operand()?),
			OpCode::Add => Instruction::Add,
			OpCode::Sub => Instruction::Sub,
			OpCode::Mul => Instruction::Mul,
			OpCode::Div => Instruction::Div,
			OpCode::PushInt => Instruction::PushInt(self.disassemble_i32_operand()?),
			OpCode::Ret => Instruction::Ret,
			OpCode::CallGlob => Instruction::CallGlob(self.disassemble_callglob_operands()?),
			OpCode::Jz => Instruction::Jz(self.disassemble_u32_operand()?),
			OpCode::Test => Instruction::Test,
			OpCode::GetVar => Instruction::GetVar(self.disassemble_variable_operand()?),
			OpCode::SetVar => Instruction::SetVar(self.disassemble_variable_operand()?),
			OpCode::PushVal => Instruction::PushVal(self.disassemble_value_operand()?),
			OpCode::DbgFile => Instruction::DbgFile(self.disassemble_string_operand()?),
			OpCode::DbgLine => Instruction::DbgLine(self.disassemble_u32_operand()?),
			_ => return Err(UnknownOp(opcode)),
		};

		Ok((starting_offset, self.current_offset - 1, res))
	}

	fn disassemble_switch_operand(&mut self) -> Result<Switch, DisassembleError> {
		let mut cases = vec![];
		
		for _ in 0..self.disassemble_u32_operand()? {
			cases.push((
				self.disassemble_value_operand()?,
				self.disassemble_loc_operand()?,
			))
		}

		Ok(Switch{
			default: self.disassemble_loc_operand()?,
			cases
		})
	}

	fn disassemble_proc_operand(&mut self) -> Result<Proc, DisassembleError> {
		let id = raw_types::procs::ProcId(self.disassemble_u32_operand()?);
		Proc::from_id(id).ok_or(InvalidProcId)
	}

	fn disassemble_callglob_operands(&mut self) -> Result<Call, DisassembleError> {
		let args = self.disassemble_param_count_operand()?;
		let proc = self.disassemble_proc_operand()?;

		Ok(Call {
			args, proc
		})
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
			AccessModifier::Args => {
				Ok(Variable::Args)
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
			AccessModifier::Cache2 => {
				Ok(Variable::Cache2)
			},
			AccessModifier::Cache3 => {
				Ok(Variable::Cache3)
			},
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
	
	// TODO: big mess
	fn disassemble_variable_field_chain(&mut self) -> Result<Variable, DisassembleError>
	{
		// This is either a string-ref or an AccessModifier
		let param = self.peek_u32_operand()?;

		let mut fields = vec![];

		let obj =
		if AccessModifier::in_range(param) {
			let modifier = self.disassemble_access_modifier_type()?;
			self.disassemble_access_modifier_operands(modifier)?	
		} else {
			fields.push(self.disassemble_string_operand()?);
			Variable::Cache
		};
	
		loop {
			// This is either a string-ref. AccessModifier::Field or AccessModifier::Initial
			let data = self.peek_u32_operand()?;

			if AccessModifier::in_range(data) {
				match self.disassemble_access_modifier_type()? {
					AccessModifier::Field => {

						// HACK: We need to rethink this whole fn
						if self.peek_u32_operand()? == AccessModifier::Global as u32 {
							self.disassemble_u32_operand()?;
							fields.push(self.disassemble_variable_name_operand()?);
							continue;
						}

						fields.push(self.disassemble_string_operand()?);
						continue;
					}
					// Initial is always last (i think,) so just grab the last string and ret
					AccessModifier::Initial => {
						fields.push(self.disassemble_string_operand()?);
						return Ok(Variable::InitialField(Box::new(obj), fields));
					}
					// Similar to initial
					AccessModifier::Proc | AccessModifier::Proc2 => {
						let proc = self.disassemble_proc_operand()?;
						return Ok(Variable::StaticProcField(Box::new(obj), fields, proc));
					}
					AccessModifier::SrcProc | AccessModifier::SrcProc2 => {
						let proc = self.disassemble_string_operand()?;
						return Ok(Variable::RuntimeProcField(Box::new(obj), fields, proc));
					}
					_ => return Err(UnknownAccessModifier)
				}
			}

			// We hit our last key
			fields.push(self.disassemble_string_operand()?);
			return Ok(Variable::Field(Box::new(obj), fields));
		}
	}
	
	fn disassemble_variable_operand(&mut self) -> Result<Variable, DisassembleError>
	{
		// This is either a string-ref or an AccessModifier
		let param = self.peek_u32_operand()?;

		if !AccessModifier::in_range(param) {
			let fields = vec![self.disassemble_string_operand()?];
			return Ok(Variable::Field(Box::new(Variable::Cache), fields));
		}

		let modifier = self.disassemble_access_modifier_type()?;
	
		match modifier {
			AccessModifier::Proc => {
				let proc = self.disassemble_proc_operand()?;
				Ok(Variable::StaticProcField(Box::new(Variable::Cache), vec![], proc))
			},
			AccessModifier::SrcProc2 => {
				let proc = self.disassemble_string_operand()?;
				Ok(Variable::RuntimeProcField(Box::new(Variable::Cache), vec![], proc))
			},
			AccessModifier::Field => {
				Ok(self.disassemble_variable_field_chain()?)
			},
			_ => self.disassemble_access_modifier_operands(modifier),
		}
	}
	
	fn disassemble_value_operand(&mut self) -> Result<Value, DisassembleError>
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

	fn peek_u32_operand(&mut self) -> Result<u32, DisassembleError>
	{
		let operand = self.iter.peek().ok_or(UnexpectedEnd)?;
		Ok(**operand)
	}

	fn disassemble_i32_operand(&mut self) -> Result<i32, DisassembleError>
	{
		Ok(self.disassemble_u32_operand()? as i32)
	}

	fn disassemble_u32_operand(&mut self) -> Result<u32, DisassembleError>
	{
		let operand = self.next().ok_or(UnexpectedEnd)?;
		Ok(*operand)
	}

	fn disassemble_param_count_operand(&mut self) -> Result<ParamCount, DisassembleError>
	{
		Ok(ParamCount(self.disassemble_u32_operand()?))
	}

	fn disassemble_loc_operand(&mut self) -> Result<Loc, DisassembleError>
	{
		Ok(Loc(self.disassemble_u32_operand()?))
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

pub fn disassemble(proc: &Proc) -> (Vec<(u32, u32, Instruction)>, Option<DisassembleError>) {
	let bytecode = unsafe {
		let (ptr, count) = proc.bytecode();
		std::slice::from_raw_parts(ptr, count)
	};

	let mut dism = Disassembler::new(bytecode.iter());

	let mut ret = vec![];
	loop {
		match dism.disassemble_instruction() {
			Ok(ins_data) => {
				if let Instruction::End() = ins_data.2 {
					ret.push(ins_data);
					break;
				}
				ret.push(ins_data);
				continue;
			},
			Err(e) => {
				return (ret, Some(e))
			}
		}
	}

	// TODO: Error

	(ret, None)
}
