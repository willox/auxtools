; for i686-pc-windows-msvc
.MODEL FLAT, C
.CODE

EXTERN _execute_instruction_original : DWORD
extern _handle_instruction : DWORD

; EDI = [CURRENT_EXECUTION_CONTEXT]
_execute_instruction_hook PROC
  ; Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH EAX
  PUSH ECX
  PUSH EDI
  call _handle_instruction
  MOV EDI, EAX
  ADD ESP, 04h
  POP ECX
  POP EAX

  ; Jump to BYOND's default do_instruction.
  MOV EAX, _execute_instruction_original
  JMP EAX
_execute_instruction_hook ENDP

END
