; for i686-pc-windows-msvc
.MODEL FLAT, C
.CODE

EXTERN execute_instruction_original : PTR
handle_instruction PROTO, opcode: DWORD

; EDI = [CURRENT_EXECUTION_CONTEXT]
execute_instruction_hook PROC PUBLIC
  ; Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH EAX
  PUSH ECX
  INVOKE handle_instruction, EDI
  MOV EDI, EAX
  POP ECX
  POP EAX

  ; Jump to BYOND's default do_instruction.
  MOV EAX, execute_instruction_original
  JMP EAX
execute_instruction_hook ENDP

END
