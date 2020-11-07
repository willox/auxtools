pub mod instructions;
pub mod opcodes;

use std::iter::Peekable;

pub use instructions::*;

use crate::raw_types;
use crate::Proc;
use crate::StringRef;
use crate::Value;
use opcodes::{AccessModifier, OpCode};

#[derive(Debug, PartialEq)]
pub enum DisassembleError {
	UnexpectedEnd,
	UnknownOp(OpCode),
	UnknownAccessModifier,
	InvalidProcId,
	UnknownIsInOperand(u32),
	UnknownRange,
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
			current_offset: 0,
		}
	}

	fn next(&mut self) -> Option<&u32> {
		self.current_offset += 1;
		self.iter.next()
	}

	fn disassemble_instruction(&mut self) -> Result<(u32, u32, Instruction), DisassembleError> {
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
			OpCode::IsLoc => Instruction::IsLoc,
			OpCode::IsMob => Instruction::IsMob,
			OpCode::IsObj => Instruction::IsObj,
			OpCode::IsArea => Instruction::IsArea,
			OpCode::IsTurf => Instruction::IsTurf,
			OpCode::IsIcon => Instruction::IsIcon,
			OpCode::IsMovable => Instruction::IsMovable,
			OpCode::IsFile => Instruction::IsFile,
			OpCode::View => Instruction::View,
			OpCode::OView => Instruction::OView,
			OpCode::Viewers => Instruction::Viewers,
			OpCode::OViewers => Instruction::OViewers,
			OpCode::Hearers => Instruction::Hearers,
			OpCode::OHearers => Instruction::OHearers,
			OpCode::Alert => Instruction::Alert,
			OpCode::CheckNum => Instruction::CheckNum,
			OpCode::ListGet => Instruction::ListGet,
			OpCode::ListSet => Instruction::ListSet,
			OpCode::BeginListSetExpr => Instruction::BeginListSetExpr,
			OpCode::Teq => Instruction::Teq,
			OpCode::Tne => Instruction::Tne,
			OpCode::Tl => Instruction::Tl,
			OpCode::Tg => Instruction::Tg,
			OpCode::Tle => Instruction::Tle,
			OpCode::Tge => Instruction::Tge,
			OpCode::TestNotEquiv => Instruction::TestNotEquiv,
			OpCode::GetFlag => Instruction::GetFlag,
			OpCode::Not => Instruction::Not,
			OpCode::Abs => Instruction::Abs,
			OpCode::Clamp => Instruction::Clamp,
			OpCode::Sqrt => Instruction::Sqrt,
			OpCode::Pow => Instruction::Pow,
			OpCode::Sin => Instruction::Sin,
			OpCode::Cos => Instruction::Cos,
			OpCode::Tan => Instruction::Tan,
			OpCode::ArcSin => Instruction::ArcSin,
			OpCode::ArcCos => Instruction::ArcCos,
			OpCode::ArcTan => Instruction::ArcTan,
			OpCode::ArcTan2 => Instruction::ArcTan2,
			OpCode::Text2Path => Instruction::Text2Path,
			OpCode::NewArgList => Instruction::NewArgList,
			OpCode::ReplaceText => Instruction::ReplaceText,
			OpCode::ReplaceTextEx => Instruction::ReplaceTextEx,
			OpCode::FindLastText => Instruction::FindLastText,
			OpCode::Shell => Instruction::Shell,
			OpCode::FExists => Instruction::FExists,
			OpCode::File2Text => Instruction::File2Text,
			OpCode::Text2File => Instruction::Text2File,
			OpCode::Params2List => Instruction::Params2List,
			OpCode::List2Params => Instruction::List2Params,
			OpCode::Index => Instruction::Index,
			OpCode::FList => Instruction::FList,
			OpCode::FCopy => Instruction::FCopy,
			OpCode::FCopyRsc => Instruction::FCopyRsc,
			OpCode::FDel => Instruction::FDel,
			OpCode::SplitText => Instruction::SplitText,
			OpCode::JoinText => Instruction::JoinText,
			OpCode::UpperText => Instruction::UpperText,
			OpCode::LowerText => Instruction::LowerText,
			OpCode::Ascii2Text => Instruction::Ascii2Text,
			OpCode::Text2Ascii => Instruction::Text2Ascii,
			OpCode::Text2Num => Instruction::Text2Num,
			OpCode::Num2Text => Instruction::Num2Text,
			OpCode::Text2NumRadix => Instruction::Text2NumRadix,
			OpCode::Num2TextRadix => Instruction::Num2TextRadix,
			OpCode::Num2TextSigFigs => Instruction::Num2TextSigFigs,
			OpCode::Length => Instruction::Length,
			OpCode::LengthChar => Instruction::LengthChar,
			OpCode::SpanTextChar => Instruction::SpanTextChar,
			OpCode::NonSpanTextChar => Instruction::NonSpanTextChar,
			OpCode::CopyText => Instruction::CopyText,
			OpCode::CmpText => Instruction::CmpText,
			OpCode::FindText => Instruction::FindText,
			OpCode::FindTextEx => Instruction::FindTextEx,
			OpCode::SortText => Instruction::SortText(self.disassemble_param_count_operand()?),
			OpCode::SortTextEx => Instruction::SortTextEx(self.disassemble_param_count_operand()?),
			OpCode::FindTextChar => Instruction::FindTextChar,
			OpCode::CopyTextChar => Instruction::CopyTextChar,
			OpCode::ReplaceTextChar => Instruction::ReplaceTextChar,
			OpCode::Time2Text => Instruction::Time2Text,
			OpCode::Md5 => Instruction::Md5,
			OpCode::CKey => Instruction::CKey,
			OpCode::CKeyEx => Instruction::CKeyEx,
			OpCode::Sleep => Instruction::Sleep,
			OpCode::Spawn => Instruction::Spawn(self.disassemble_loc_operand()?),
			OpCode::NullCacheMaybe => Instruction::NullCacheMaybe,
			OpCode::PushToCache => Instruction::PushToCache,
			OpCode::PopFromCache => Instruction::PopFromCache,
			OpCode::Flick => Instruction::Flick,
			OpCode::LocatePos => Instruction::LocatePos,
			OpCode::LocateRef => Instruction::LocateRef,
			OpCode::LocateType => Instruction::LocateType,
			OpCode::Rgb => Instruction::Rgb,
			OpCode::Rgba => Instruction::Rgba,
			OpCode::BoundsDist => Instruction::BoundsDist,
			OpCode::HasCall => Instruction::HasCall,
			OpCode::CallLib => Instruction::CallLib(self.disassemble_param_count_operand()?),
			OpCode::CallPath => Instruction::CallPath(self.disassemble_param_count_operand()?),
			OpCode::CallParent => Instruction::CallParent,
			OpCode::CallParentArgList => Instruction::CallParentArgList,
			OpCode::CallParentArgs => {
				Instruction::CallParentArgs(self.disassemble_param_count_operand()?)
			}
			OpCode::CallSelf => Instruction::CallSelf,
			OpCode::CallSelfArgList => Instruction::CallSelfArgList,
			OpCode::CallSelfArgs => {
				Instruction::CallSelfArgs(self.disassemble_param_count_operand()?)
			}
			OpCode::CallPathArgList => Instruction::CallPathArgList,
			OpCode::CallNameArgList => Instruction::CallNameArgList,
			OpCode::Block => Instruction::Block,
			OpCode::BrowseOpt => Instruction::BrowseOpt,
			OpCode::BrowseRsc => Instruction::BrowseRsc,

			OpCode::Turn => Instruction::Turn,
			OpCode::Step => Instruction::Step,
			OpCode::StepTo => Instruction::StepTo,
			OpCode::StepAway => Instruction::StepAway,
			OpCode::StepTowards => Instruction::StepTowards,
			OpCode::StepRand => Instruction::StepRand,
			OpCode::StepToSpeed => Instruction::StepToSpeed,
			OpCode::StepSpeed => Instruction::StepSpeed,
			OpCode::Walk => Instruction::Walk,
			OpCode::WalkTo => Instruction::WalkTo,
			OpCode::WalkAway => Instruction::WalkAway,
			OpCode::WalkTowards => Instruction::WalkTowards,
			OpCode::WalkRand => Instruction::WalkRand,
			OpCode::GetStep => Instruction::GetStep,
			OpCode::GetStepTo => Instruction::GetStepTo,
			OpCode::GetStepAway => Instruction::GetStepAway,
			OpCode::GetStepTowards => Instruction::GetStepTowards,
			OpCode::GetStepRand => Instruction::GetStepRand,
			OpCode::GetDist => Instruction::GetDist,
			OpCode::GetDir => Instruction::GetDir,

			OpCode::IsIn => Instruction::IsIn(match self.disassemble_u32_operand()? {
				0x0B => IsInOperand::Range,
				0x05 => IsInOperand::Value,
				x => return Err(UnknownIsInOperand(x)),
			}),

			OpCode::Range => {
				if self.disassemble_u32_operand()? != 0xAE {
					return Err(UnknownRange);
				}
				Instruction::Range
			}

			OpCode::Orange => {
				if self.disassemble_u32_operand()? != 0xAE {
					return Err(UnknownRange);
				}
				Instruction::Orange
			}

			OpCode::ForRange => Instruction::ForRange(
				self.disassemble_loc_operand()?,
				self.disassemble_variable_operand()?,
			),

			OpCode::ForRangeStepSetup => Instruction::ForRangeStepSetup,
			OpCode::ForRangeStep => Instruction::ForRangeStep(
				self.disassemble_loc_operand()?,
				self.disassemble_variable_operand()?,
			),

			OpCode::Min => Instruction::Min(self.disassemble_param_count_operand()?),
			OpCode::Max => Instruction::Max(self.disassemble_param_count_operand()?),
			OpCode::MinList => Instruction::MinList,
			OpCode::MaxList => Instruction::MaxList,
			OpCode::Inc => Instruction::Inc(self.disassemble_variable_operand()?),
			OpCode::Dec => Instruction::Dec(self.disassemble_variable_operand()?),
			OpCode::PreInc => Instruction::PreInc(self.disassemble_variable_operand()?),
			OpCode::PostInc => Instruction::PostInc(self.disassemble_variable_operand()?),
			OpCode::PreDec => Instruction::PreDec(self.disassemble_variable_operand()?),
			OpCode::PostDec => Instruction::PostDec(self.disassemble_variable_operand()?),
			OpCode::TypesOf => Instruction::TypesOf(self.disassemble_param_count_operand()?),
			OpCode::Switch => Instruction::Switch(self.disassemble_switch_operand()?),
			OpCode::PickSwitch => Instruction::PickSwitch(self.disassemble_pick_switch_operand()?),
			OpCode::SwitchRange => {
				Instruction::SwitchRange(self.disassemble_switch_range_operand()?)
			}
			OpCode::Jmp => Instruction::Jmp(self.disassemble_loc_operand()?),
			OpCode::Jmp2 => Instruction::Jmp2(self.disassemble_loc_operand()?),
			OpCode::Jnz2 => Instruction::Jnz2(self.disassemble_loc_operand()?),
			OpCode::Jz2 => Instruction::Jz2(self.disassemble_loc_operand()?),
			OpCode::Ref => Instruction::Ref,
			OpCode::Animate => Instruction::Animate,
			OpCode::NullAnimate => Instruction::NullAnimate,
			OpCode::AugAdd => Instruction::AugAdd(self.disassemble_variable_operand()?),
			OpCode::AugSub => Instruction::AugSub(self.disassemble_variable_operand()?),
			OpCode::AugMul => Instruction::AugMul(self.disassemble_variable_operand()?),
			OpCode::AugDiv => Instruction::AugDiv(self.disassemble_variable_operand()?),
			OpCode::AugMod => Instruction::AugMod(self.disassemble_variable_operand()?),
			OpCode::AugBand => Instruction::AugBand(self.disassemble_variable_operand()?),
			OpCode::AugBor => Instruction::AugBor(self.disassemble_variable_operand()?),
			OpCode::AugXor => Instruction::AugXor(self.disassemble_variable_operand()?),
			OpCode::AugLShift => Instruction::AugLShift(self.disassemble_variable_operand()?),
			OpCode::AugRShift => Instruction::AugRShift(self.disassemble_variable_operand()?),
			OpCode::Input => Instruction::Input(
				self.disassemble_u32_operand()?,
				self.disassemble_u32_operand()?,
				self.disassemble_u32_operand()?,
			),
			OpCode::InputColor => Instruction::InputColor(
				self.disassemble_u32_operand()?,
				self.disassemble_u32_operand()?,
				self.disassemble_u32_operand()?,
			),
			OpCode::PromptCheck => Instruction::PromptCheck,
			OpCode::IterLoad => Instruction::IterLoad(
				self.disassemble_u32_operand()?,
				self.disassemble_u32_operand()?,
			),
			OpCode::IterNext => Instruction::IterNext,
			OpCode::IterPush => Instruction::IterPush,
			OpCode::IterPop => Instruction::IterPop,
			OpCode::Format => Instruction::Format(
				self.disassemble_string_operand()?,
				self.disassemble_param_count_operand()?,
			),
			OpCode::OutputFormat => Instruction::OutputFormat(
				self.disassemble_string_operand()?,
				self.disassemble_param_count_operand()?,
			),
			OpCode::JsonEncode => Instruction::JsonEncode,
			OpCode::JsonDecode => Instruction::JsonDecode,
			OpCode::HtmlEncode => Instruction::HtmlEncode,
			OpCode::HtmlDecode => Instruction::HtmlDecode,
			OpCode::FilterNewArgList => Instruction::FilterNewArgList,
			OpCode::UrlEncode => Instruction::UrlEncode,
			OpCode::UrlDecode => Instruction::UrlDecode,
			OpCode::JmpOr => Instruction::JmpOr(self.disassemble_loc_operand()?),
			OpCode::JmpAnd => Instruction::JmpAnd(self.disassemble_loc_operand()?),
			OpCode::JmpIfNull => Instruction::JmpIfNull(self.disassemble_loc_operand()?),
			OpCode::JmpIfNull2 => Instruction::JmpIfNull2(self.disassemble_loc_operand()?),
			OpCode::NewAssocList => {
				Instruction::NewAssocList(self.disassemble_param_count_operand()?)
			}
			OpCode::Crash => Instruction::Crash,
			OpCode::EmptyList => Instruction::EmptyList,
			OpCode::NewList => Instruction::NewList(self.disassemble_u32_operand()?),
			OpCode::Try => Instruction::Try(self.disassemble_loc_operand()?),
			OpCode::Catch => Instruction::Catch(self.disassemble_loc_operand()?),
			OpCode::CallName => Instruction::CallName(self.disassemble_param_count_operand()?),
			OpCode::End => Instruction::End(),
			OpCode::Pick => Instruction::Pick,
			OpCode::PickProb => {
				let mut locs = vec![];
				for _ in 0..self.disassemble_u32_operand()? {
					locs.push(self.disassemble_loc_operand()?);
				}
				Instruction::PickProb(PickProb { locs })
			}
			OpCode::Rand => Instruction::Rand,
			OpCode::RandSeed => Instruction::RandSeed,
			OpCode::Prob => Instruction::Prob,
			OpCode::RandRange => Instruction::RandRange,
			OpCode::NewImageArgList => Instruction::NewImageArgList,
			OpCode::NewImageArgs => {
				Instruction::NewImageArgs(self.disassemble_param_count_operand()?)
			}
			OpCode::New => Instruction::New(self.disassemble_param_count_operand()?),
			OpCode::MatrixNew => Instruction::MatrixNew(self.disassemble_param_count_operand()?),
			OpCode::Database => Instruction::Database(self.disassemble_param_count_operand()?),
			OpCode::RegexNew => Instruction::RegexNew(self.disassemble_param_count_operand()?),
			OpCode::IconNew => Instruction::IconNew(self.disassemble_param_count_operand()?),
			OpCode::IconStates => Instruction::IconStates,
			OpCode::IconStatesMode => Instruction::IconStatesMode,
			OpCode::TurnOrFlipIcon => Instruction::TurnOrFlipIcon {
				filter_mode: self.disassemble_u32_operand()?,
				var: self.disassemble_variable_operand()?,
			},
			OpCode::ShiftIcon => Instruction::ShiftIcon(self.disassemble_variable_operand()?),
			OpCode::IconIntensity => {
				Instruction::IconIntensity(self.disassemble_variable_operand()?)
			}
			OpCode::IconBlend => Instruction::IconBlend(self.disassemble_variable_operand()?),
			OpCode::IconSwapColor => {
				Instruction::IconSwapColor(self.disassemble_variable_operand()?)
			}
			OpCode::IconDrawBox => Instruction::IconDrawBox(self.disassemble_variable_operand()?),
			OpCode::IconInsert => Instruction::IconInsert(self.disassemble_param_count_operand()?),
			OpCode::IconMapColors => {
				Instruction::IconMapColors(self.disassemble_param_count_operand()?)
			}
			OpCode::IconScale => Instruction::IconScale(self.disassemble_variable_operand()?),
			OpCode::IconCrop => Instruction::IconCrop(self.disassemble_variable_operand()?),
			OpCode::IconGetPixel => {
				Instruction::IconGetPixel(self.disassemble_param_count_operand()?)
			}
			OpCode::IconSize => Instruction::IconSize,
			OpCode::NewImage => Instruction::NewImage,
			OpCode::Pop => Instruction::Pop,
			OpCode::PopN => Instruction::PopN(self.disassemble_u32_operand()?),
			OpCode::Stat => Instruction::Stat,
			OpCode::Output => Instruction::Output,
			OpCode::OutputFtp => Instruction::OutputFtp,
			OpCode::OutputRun => Instruction::OutputRun,
			OpCode::Read => Instruction::Read,
			OpCode::WinOutput => Instruction::WinOutput,
			OpCode::WinSet => Instruction::WinSet,
			OpCode::WinGet => Instruction::WinGet,
			OpCode::WinShow => Instruction::WinShow,
			OpCode::WinClone => Instruction::WinClone,
			OpCode::WinExists => Instruction::WinExists,
			OpCode::CallNoReturn => Instruction::CallNoReturn(
				self.disassemble_variable_operand()?,
				self.disassemble_u32_operand()?,
			),
			OpCode::Call => Instruction::Call(
				self.disassemble_variable_operand()?,
				self.disassemble_u32_operand()?,
			),
			OpCode::CallGlobalArgList => {
				Instruction::CallGlobalArgList(self.disassemble_proc_operand()?)
			}
			OpCode::RollStr => Instruction::RollStr,
			OpCode::UnaryNeg => Instruction::UnaryNeg,
			OpCode::Add => Instruction::Add,
			OpCode::Sub => Instruction::Sub,
			OpCode::Mul => Instruction::Mul,
			OpCode::Div => Instruction::Div,
			OpCode::Mod => Instruction::Mod,
			OpCode::Log => Instruction::Log,
			OpCode::Log10 => Instruction::Log10,
			OpCode::Round => Instruction::Round,
			OpCode::RoundN => Instruction::RoundN,
			OpCode::Band => Instruction::Band,
			OpCode::Bor => Instruction::Bor,
			OpCode::Bxor => Instruction::Bxor,
			OpCode::Bnot => Instruction::Bnot,
			OpCode::LShift => Instruction::LShift,
			OpCode::RShift => Instruction::RShift,
			OpCode::PushInt => Instruction::PushInt(self.disassemble_i32_operand()?),
			OpCode::Ret => Instruction::Ret,
			OpCode::CallGlob => Instruction::CallGlob(self.disassemble_callglob_operands()?),
			OpCode::Jz => Instruction::Jz(self.disassemble_u32_operand()?),
			OpCode::Test => Instruction::Test,
			OpCode::Del => Instruction::Del,
			OpCode::Link => Instruction::Link,
			OpCode::GetVar => Instruction::GetVar(self.disassemble_variable_operand()?),
			OpCode::SetVar => Instruction::SetVar(self.disassemble_variable_operand()?),
			//OpCode::SetVarRet => Instruction::SetVarRet(self.disassemble_variable_operand()?),
			OpCode::SetVarExpr => Instruction::SetVarExpr(self.disassemble_variable_operand()?),
			OpCode::PushVal => Instruction::PushVal(self.disassemble_value_operand()?),
			OpCode::DbgFile => Instruction::DbgFile(self.disassemble_string_operand()?),
			OpCode::DbgLine => Instruction::DbgLine(self.disassemble_u32_operand()?),

			OpCode::DebugBreak => {
				// Allow peek to fail (in case we have a DebugBreak at the end of a proc)
				loop {
					match self.peek_u32_operand() {
						Ok(operand) => {
							if operand == opcodes::DEBUG_BREAK_OPERAND {
								self.disassemble_u32_operand()?;
							} else {
								break;
							}
						}

						Err(_) => break,
					}
				}

				// TODO: get the original from the debug server? Probably not.
				Instruction::DebugBreak
			}

			_ => return Err(UnknownOp(opcode)),
		};

		Ok((starting_offset, self.current_offset - 1, res))
	}

	fn disassemble_switch_range_operand(&mut self) -> Result<SwitchRange, DisassembleError> {
		let mut cases = vec![];
		let mut range_cases = vec![];

		for _ in 0..self.disassemble_u32_operand()? {
			let min = self.disassemble_value_operand()?;
			let max = self.disassemble_value_operand()?;
			let loc = self.disassemble_loc_operand()?;

			range_cases.push((min, max, loc));
		}

		for _ in 0..self.disassemble_u32_operand()? {
			let val = self.disassemble_value_operand()?;
			let loc = self.disassemble_loc_operand()?;

			cases.push((val, loc));
		}

		Ok(SwitchRange {
			default: self.disassemble_loc_operand()?,
			cases,
			range_cases,
		})
	}

	fn disassemble_pick_switch_operand(&mut self) -> Result<PickSwitch, DisassembleError> {
		let mut cases = vec![];

		for _ in 0..self.disassemble_u32_operand()? {
			cases.push((
				self.disassemble_u32_operand()?,
				self.disassemble_loc_operand()?,
			))
		}

		Ok(PickSwitch {
			default: self.disassemble_loc_operand()?,
			cases,
		})
	}

	fn disassemble_switch_operand(&mut self) -> Result<Switch, DisassembleError> {
		let mut cases = vec![];

		for _ in 0..self.disassemble_u32_operand()? {
			cases.push((
				self.disassemble_value_operand()?,
				self.disassemble_loc_operand()?,
			))
		}

		Ok(Switch {
			default: self.disassemble_loc_operand()?,
			cases,
		})
	}

	fn disassemble_proc_operand(&mut self) -> Result<Proc, DisassembleError> {
		let id = raw_types::procs::ProcId(self.disassemble_u32_operand()?);
		Proc::from_id(id).ok_or(InvalidProcId)
	}

	fn disassemble_callglob_operands(&mut self) -> Result<Call, DisassembleError> {
		let args = self.disassemble_param_count_operand()?;
		let proc = self.disassemble_proc_operand()?;

		Ok(Call { args, proc })
	}

	fn disassemble_access_modifier_type(&mut self) -> Result<AccessModifier, DisassembleError> {
		let operand = self.disassemble_u32_operand()?;
		Ok(unsafe { std::mem::transmute(operand) })
	}

	fn disassemble_access_modifier_operands(
		&mut self,
		modifier: AccessModifier,
	) -> Result<Variable, DisassembleError> {
		match modifier {
			AccessModifier::Null => Ok(Variable::Null),
			AccessModifier::World => Ok(Variable::World),
			AccessModifier::Usr => Ok(Variable::Usr),
			AccessModifier::Src => Ok(Variable::Src),
			AccessModifier::Args => Ok(Variable::Args),
			AccessModifier::Dot => Ok(Variable::Dot),
			AccessModifier::Cache => Ok(Variable::Cache),
			AccessModifier::Arg => Ok(Variable::Arg(self.disassemble_u32_operand()?)),
			AccessModifier::Local => Ok(Variable::Local(self.disassemble_u32_operand()?)),
			AccessModifier::Cache2 => Ok(Variable::Cache2),
			AccessModifier::Cache3 => Ok(Variable::Cache3),
			AccessModifier::Global => {
				Ok(Variable::Global(self.disassemble_variable_name_operand()?))
			}
			// These modifiers have potentially recursive behaviour and are handled outside of this method
			AccessModifier::Field => Err(UnknownAccessModifier),
			AccessModifier::Initial => Err(UnknownAccessModifier),
			_ => Err(UnknownAccessModifier),
		}
	}

	// TODO: big mess
	fn disassemble_variable_field_chain(&mut self) -> Result<Variable, DisassembleError> {
		// This is either a string-ref or an AccessModifier
		let param = self.peek_u32_operand()?;

		let mut fields = vec![];

		let obj = if AccessModifier::in_range(param) {
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
					_ => return Err(UnknownAccessModifier),
				}
			}

			// We hit our last key
			fields.push(self.disassemble_string_operand()?);
			return Ok(Variable::Field(Box::new(obj), fields));
		}
	}

	fn disassemble_variable_operand(&mut self) -> Result<Variable, DisassembleError> {
		// This is either a string-ref or an AccessModifier
		let param = self.peek_u32_operand()?;

		if !AccessModifier::in_range(param) {
			let fields = vec![self.disassemble_string_operand()?];
			return Ok(Variable::Field(Box::new(Variable::Cache), fields));
		}

		let modifier = self.disassemble_access_modifier_type()?;

		match modifier {
			AccessModifier::Initial => {
				let var = self.disassemble_string_operand()?;
				Ok(Variable::InitialField(Box::new(Variable::Cache), vec![var]))
			}
			AccessModifier::Proc | AccessModifier::Proc2 => {
				let proc = self.disassemble_proc_operand()?;
				Ok(Variable::StaticProcField(
					Box::new(Variable::Cache),
					vec![],
					proc,
				))
			}
			AccessModifier::SrcProc | AccessModifier::SrcProc2 => {
				let proc = self.disassemble_string_operand()?;
				Ok(Variable::RuntimeProcField(
					Box::new(Variable::Cache),
					vec![],
					proc,
				))
			}
			AccessModifier::Field => Ok(self.disassemble_variable_field_chain()?),
			_ => self.disassemble_access_modifier_operands(modifier),
		}
	}

	fn disassemble_value_operand(&mut self) -> Result<Value, DisassembleError> {
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

	fn peek_u32_operand(&mut self) -> Result<u32, DisassembleError> {
		let operand = self.iter.peek().ok_or(UnexpectedEnd)?;
		Ok(**operand)
	}

	fn disassemble_i32_operand(&mut self) -> Result<i32, DisassembleError> {
		Ok(self.disassemble_u32_operand()? as i32)
	}

	fn disassemble_u32_operand(&mut self) -> Result<u32, DisassembleError> {
		let operand = self.next().ok_or(UnexpectedEnd)?;
		Ok(*operand)
	}

	fn disassemble_param_count_operand(&mut self) -> Result<ParamCount, DisassembleError> {
		Ok(ParamCount(self.disassemble_u32_operand()?))
	}

	fn disassemble_loc_operand(&mut self) -> Result<Loc, DisassembleError> {
		Ok(Loc(self.disassemble_u32_operand()?))
	}

	fn disassemble_variable_name_operand(&mut self) -> Result<StringRef, DisassembleError> {
		let operand = self.next().ok_or(UnexpectedEnd)?;
		let str = unsafe { StringRef::from_variable_id(raw_types::strings::VariableId(*operand)) };

		Ok(str)
	}

	fn disassemble_string_operand(&mut self) -> Result<StringRef, DisassembleError> {
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
				ret.push(ins_data);
				continue;
			}
			Err(e) => {
				if e != Finished {
					return (ret, Some(e));
				}
				break;
			}
		}
	}

	// TODO: Error

	(ret, None)
}
