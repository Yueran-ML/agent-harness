<!--
For maintainers (this comment is not shipped: electron/engine-bundle.mjs drops it, and appends
the rest to resources/engine/THIRD_PARTY_ENGINE.md when it brings the DirectX Shader Compiler).
The quotation is of the Microsoft Software License Terms of the Windows SDK 10.0.26100
(Licenses\10.0.26100.0\sdk_license.rtf, section 2.a), and the terms for the user are written to
meet its requirement that end users agree to terms that protect the Distributable Code at least
as much (2.a.ii), taken from its sections 2.a.iii, 7, 14 and 15. Check them again when the SDK that
the package takes them from changes its terms.
-->

## DirectX Shader Compiler

`dxcompiler.dll` and `dxil.dll` are the DirectX Shader Compiler of Microsoft
(https://github.com/microsoft/DirectXShaderCompiler), (c) Microsoft Corporation. All rights
reserved. They are not part of Columbina and are not under its MIT license.

- **Where they come from.** They are copied unmodified from `Redist\D3D\x64` of the Microsoft
  Windows Software Development Kit (SDK) for Windows. Their version is given below under *This
  build of the DirectX Shader Compiler*.
- **How the engine uses them.** The engine loads `dxcompiler.dll` by its full path from this
  folder to compile its shaders for Direct3D 12, and `dxcompiler.dll` loads `dxil.dll` beside the
  engine to sign them. Without them the engine compiles its shaders with FXC, which is part of
  Windows. `COLUMBINA_DX12_COMPILER=fxc` makes it do so with them too.
- **License.** Microsoft licenses these files under the Microsoft Software License Terms for the
  Microsoft Windows Software Development Kit, which make them Distributable Code: files of the
  SDK's REDIST.TXT list, which a developer may distribute with programs of their own. Microsoft,
  not Columbina, licenses them to you. Those terms say, of Distributable Code:

  > Right to Use and Distribute. The code and test files listed below are “Distributable Code”.
  > REDIST.TXT Files. You may copy and distribute the object code form of code listed in
  > REDIST.TXT files plus the files listed on the REDIST.TXT list located at
  > <http://go.microsoft.com/fwlink/?LinkId=524842>. […]
  >
  > Distribution Restrictions. You may not
  > - Alter any copyright, trademark or patent notice in the Distributable Code;
  > - Use Microsoft’s trademarks in your programs’ names or in a way that suggests your programs
  >   come from or are endorsed by Microsoft; […]
  > - Distribute Distributable Code to run on a platform other than the Microsoft operating
  >   system platform;
  > - Include Distributable Code in malicious, deceptive or unlawful programs; […]

- **Terms for you.** You receive `dxcompiler.dll` and `dxil.dll` on these conditions, which
  protect them as much as Microsoft's terms do. You may use them only as part of Columbina, on
  Windows. You may not modify them, alter any copyright, trademark or patent notice in them, work
  around any technical limitation in them, or reverse engineer, decompile or disassemble them,
  except and only to the extent that applicable law expressly permits. You may pass them on only
  unmodified, together with Columbina, and on these same conditions. No right to Microsoft's
  trademarks is given. They are provided "as is": Microsoft gives no express warranties,
  guarantees or conditions, and, to the extent permitted under your local laws, excludes the
  implied warranties of merchantability, fitness for a particular purpose and non-infringement.
  You can recover from Microsoft and its suppliers only direct damages up to U.S. $5.00, and no
  other damages. You may have rights under the laws of your country that these conditions cannot
  change.
- **Third-party notices.** The DirectX Shader Compiler contains LLVM and Clang, under the
  University of Illinois/NCSA Open Source License, and other code under its own license. Their
  notices, as the Windows SDK gives them, are `licenses/DirectXShaderCompiler/THIRD-PARTY-NOTICES.txt`.
