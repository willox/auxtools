// for i686-unknown-linux-gnu
.intel_syntax noprefix
.global execute_instruction_hook_516
.extern execute_instruction_original
.extern handle_instruction

// this just stores some pretty important registers before calling the actual function
execute_instruction_hook_516:
  //SUB ESP, 0x04
  PUSH EAX
  PUSH EBX
  PUSH ECX
  PUSH EDX
  PUSH EBP

  MOV EBP, ESP
  AND ESP, 0xfffffff0 // clear last 4 bits, align for 16-bytes
  SUB ESP, 0x10
  MOVDQA [ESP], XMM0
  SUB ESP, 0x10

  CALL handle_instruction

  MOVDQA XMM0, [ESP + 0x10]
  MOV ESP, EBP

  POP EBP
  POP EDX
  POP ECX
  POP EBX
  POP EAX
  //ADD ESP, 0x04

  MOV EDI, execute_instruction_original // EDI is overwritten before read again, it can be used as a volatile register
  JMP EDI
