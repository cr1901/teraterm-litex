# Tera Term 4/5 Plugin for LiteX Serial Flash Loader (SFL) Protocol

Tera Term is a mature and still developed Open Source Terminal Emulator for
Windows. It has its roots in the Windows 3.x days, and has been rewritten from
Pascal to C and some C++. This plugin adds some Rust to the mix :). It augments
the [serial boot](https://github.com/enjoy-digital/litex/wiki/Load-Application-Code-To-CPU#serial-boot)
protocol used for rapid firmware development in the [LiteX](https://github.com/enjoy-digital/litex)
FPGA/SoC ecosystem.

The plugin is called `TTXLiteX.dll` to conform to Tera Term's
[naming conventions](https://teratermproject.github.io/manual/5/en/reference/sourcecode.html#module).
The 3-Clause BSD License conforms to what most Tera Term plugins
[seem to use](https://teratermproject.github.io/manual/5/en/about/copyright.html).

## Demo
The following GIF was [created](https://github.com/saitoha/seq2gif)
by running Tera Term's
[ttyrec-compatible recorder](https://teratermproject.github.io/manual/5/en/usage/ttyrec.html),
and does not necessarily reflect the fonts/glyphs used by Tera Term.

![GIF of a sample session uploading the LiteX demo firmware via the LiteX BIOS
 using my Tera Term LiteX plugin. I then run the first frame of a demo that
 prints a 3D donut rendered using ASCII characters.](assets/demo.gif)

## Installation
There are two installers; the DLL is identical in each except for default
install options (install as part of Tera Term 4 or 5). The installer will
install the plugin into the Tera Term install directory. I also provide a ZIP
file with the installers, the DLL itself, and README.md, CHANGELOG.md, etc.
If you have a portable Tera Term, you can just copy the DLL itself to the same
directory where the Tera Term binary lives and the plugin should "just work".

Only 32-bit DLLs are provided, because [only 32-bit binary releases](https://teratermproject.github.io/manual/5/en/reference/sourcecode.html#module)
of Tera Term are provided at this time. However, I have confirmed that
Tera Term, as well as my plugin, _do_ build and work just fine for 64-bit
Windows, using [MinGW64/MSYS2](https://www.msys2.org/).

## How To Use
Once you start Tera Term after installation, you will be greeted with a new
file transfer option called "LiteX" under the File > Transfer submenu:

![Picture of a Tera Term session showing a mouse cursor highlighted over the
  File > Transfer > LiteX submenu.](assets/TqRBnncyeJ.png)

When you click "LiteX" under the File > Transfer submenu, you will be greeted
with the following dialog:

![Picture of the LiteX file dialog that opens when clicking the aforementioned
  LiteX submenu. There are text edit controls for adding a firmware file
  (named "File") and a boot address (named "Boot Address"). Additionally, to
  the right of these text edits, there is a button with ellipses and an unchecked
  checkbox called "Active". A Tera Term session is in the background.](assets/ttermpro_Bkg4qsDMaR.png)

The "File" and "Boot Address" text edits correspond to the `--kernel` and
`--kernel-adr` parameters of [`litex-term`](https://github.com/enjoy-digital/litex/wiki/Load-Application-Code-To-CPU#serial-boot).
The "File" need not exist until the actual transfer takes place; clicking the
elipses will bring up an Open File for convenience. Unless you have good reason
to believe otherwise[^1], the "Boot Address" field should be set to
"the beginning of the memory region used for the Memtest during LiteX
Initialization":

![Picture of a Tera Term session showing the Memtest output from the LiteX
  BIOS. The important line displays "Memtest at 0x21010000 (64.0KiB)...".](assets/ttermpro_Eml355GWiP.png)

In the case of the above picture, "Boot Address" should be set to `0x21010000`,
or some equivalent decimal or hexadecimal number (underscores allowed for
formatting purposes).

Once "Active" is checked and you click "OK", the plugin will begin to look
to start an SFL transfer.

Because Windows doesn't like it when you modify a file that's in use, the file
is only opened just before a transfer takes place. This allows you to swap out
a fresh new copy of your firmware to upload without worrying that the
compile will fail because the file is busy[^2].

## SFL Protocol
Once an SFL transfer has been requested using the above dialog, the plugin
uses an FSM implementing the SFL protocol to send a file to a receiver. _The
plugin opens the file to send as late as possible, after the magic string has
been detected, but before the plugin sends a response._ This way, if there was
an error opening the file, the plugin will back out of the transfer.

Starting a SFL Protocol transfer relies on an [In-band](https://en.wikipedia.org/wiki/In-band_signaling)
command-response to start a transfer:

* A terminal application listen for the byte string `sL5DdSMmkekro\n` in order
  from a device that wishes to receives a file.
* When a terminal detects the above string, send the byte string
  `z6IHG7cYDID6o\n` in response[^3].

While theoretically possible without any extra user intervention, a terminal
program is unlikely to send the string `z6IHG7cYDID6o\n` in response to
receiving the bytes `sL5DdSMmkekro\n` as part of typical user operation. Thus,
SFL assumes that if the firmware receiver detects the correct response, the
user has explicitly set up their terminal to upload a firmware to the other
side of the link. In the case of the TTXLiteX plugin, committing to an upload
is indicated by checking the "Active" box and clicking "OK" in the dialog.

Once the initial command-response has been negotiated, an SFL transfer consists
of sending [Type-Length-Value](https://en.wikipedia.org/wiki/Type%E2%80%93length%E2%80%93value)
packets in order and waiting for a response for each packet. Multiple packets
can be sent before acknowledgment of all previous packets, but acknowledgment
happens in the order that packets were sent. The sent packet looks like the
following (**all multibyte fields are big-endian**, except for data payload,
which is explained below):

```
[len] [crc] [cmd] [payload]
```

* `len` is a 1-byte field indicating the length of the payload; it is the "L"
  in TLV.
* `crc` is a 2-byte field consisting of the [CRC16](https://en.wikipedia.org/wiki/Cyclic_redundancy_check)
  of the concatenation of the `cmd` _and_ `payload` fields. Without
  [getting lost in the weeds](http://www.ross.net/crc/download/crc_v3.txt)
  of CRCs, the CRC is the [same one](https://reveng.sourceforge.io/crc-catalogue/all.htm#crc.cat.crc-16-xmodem)
  as used in [XMODEM](https://en.wikipedia.org/wiki/XMODEM). [This page](https://mdfs.net/Info/Comp/Comms/CRC16.htm)
  has several basic (non-table-driven) implementations for various CPUs.
* `cmd` is a 1-byte field. It must either `0` for "Abort", `1` for "Load",
  and `2` for "Jump"; it is the "T" in TLV, and modifies the payload:
  * "Abort" stops the transfer completely, and the sender goes back to waiting
    for the receiver to send the magic string.
  * "Jump" finishes the transfer, and instructs the receiver's CPU to jump
    to the supplied address in the receiver's memory. The sender goes back
    to waiting for a magic string.
  * "Load" loads up to 251 bytes at a specified address in the receiver's
    memory. See `payload`.
* `payload` is the "V" in TLV, and can be up to 255 bytes in length:
  * "Abort" command has no payload.
  * "Jump" command has a 4-byte payload, consisting of the address for the
    receiving CPU to jump to.
  * The "Load" command payload starts with a 4-byte address, and up to 251
    bytes of data to write starting at the supplied address:

    ```
    [payload] = "[addr] [data]"
    ```
    
    The data part of the payload is written to the receiver memory _as if_ the
    writes were done one byte at a time, in order of being received.

The receiver will respond to each packet with one of 4 ASCII codes (1-byte):

* `K`- The packet was received (acKed) successfully.
* `C`- The packet had a CRC error.
* `U`- The packet's CRC was fine, but the `cmd` field was invalid.
* `E`- (`E` for "Error"?) The receiver timed out waiting for the entire packet.
  The timeout countdown is per character sent the initial `len` byte is sent. In LiteX
  BIOS's implementation, the timeout is
  [1/4 of a second](https://github.com/enjoy-digital/litex/blob/994e422d1bf3cff985490341a86c16ad90750456/litex/soc/software/bios/boot.c#L102)
  [for each character](https://github.com/enjoy-digital/litex/blob/994e422d1bf3cff985490341a86c16ad90750456/litex/soc/software/bios/boot.c#L194-L197)
  sent.

In the LiteX receiver can abort the transfer
[without notifying the sender](https://github.com/enjoy-digital/litex/blob/994e422d1bf3cff985490341a86c16ad90750456/litex/soc/software/bios/boot.c#L284-L287).
I have not implemented recovering from this case.

## Development
Development requires the most recent stable Rust compiler. Only developing
on Windows is supported at present. Follow the directions
[here](https://rustup.rs/) to install `rustup`.

If you have the [Tera Term source](https://github.com/TeraTermProject/teraterm),
you can compile Tera Term for the GNU ABI using their
[`mingw.toolchain.cmake`](https://github.com/TeraTermProject/teraterm/blob/main/mingw.toolchain.cmake)
[CMake Toolchain File](https://cmake.org/cmake/help/latest/manual/cmake-toolchains.7.html),
as well as do a 64-bit build. In particular, I've found the 64-bit GNU ABI
version of Tera Term to "just work" after a compiling, and develop against
that. However at present Tera Term is _released_ only as a 32-bit application,
presumably for max compatibility. There may be other toolchain setups that work,
but the easiest setup I've found due to [how Rust build scripts work](https://github.com/rust-lang/rust/issues/43163#issuecomment-314353725)
and [how the `windres` crate works](https://github.com/FaultyRAM/windres-rs/blob/9ac7f25dfdc40f6a0ee24e52116a73746620feb5/src/lib.rs#L45-L50)
is to install the `i686-pc-windows-msvc` _host_[^4] Rust compiler and the 32-bit
[MSVC toolchain](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
On 64-bit Windows, using your `i686-pc-windows-msvc`-hosted Rust from within a "x86 Native Tools Command Prompt"
worked for me.

Tera Term is a C/C++ codebase, while this plugin is written in Rust. Teraterm seems
unlikely to change the data structures and functions exposed to plugins
(i.e. the plugin works for Tera Term 4 _and_ 5), so I went ahead and generated
Rust bindings for the C/C++ plugin code that Rust understands, and include them
in the plugin crate. This prevents a user from needing the Tera Term source
just for small changes. If for some reason bindings need to be regenerated,
you need to [install](https://rust-lang.github.io/rust-bindgen/command-line-usage.html)
`bindgen-cli` and run it against `wrapper.h`.

I provide a [Justfile](https://github.com/casey/just) for convenience for all
of the above, so you probably also want to install `just` and take a look at
the available recipes (`just -l`).

I created the dialog using [RisohEditor](https://github.com/katahiromz/RisohEditor).
RisohEditor will simultaneously read _and write_ the provided `resource.h` to
extract constants for resource identifiers. Unfortunately, I haven't figured
out how to share `resource.h` with Rust code well, so the constants are
duplicated as needed in the plugin.

## Known Issues
* If enough failed transfers happen, the plugin FSM may go out of sync with the
  LiteX BIOS (or any other SFL receiver). Specifically, the plugin can't tell
  whether the receiving side gave up. You can reset the plugin state by:

  * Reopening the LiteX dialog.
  * Unchecking the Active box.
  * Hitting "OK".

  Once you're ready to do a transfer again, make sure to open the LiteX dialog
  again and click the Active checkbox!
* Related to above, the plugin should probably terminate the transfer on too
  many errors; it doesn't right now.
* I don't support the [JSON file](https://github.com/enjoy-digital/litex/wiki/Load-Application-Code-To-CPU#serial-boot)
  for multiple boot images yet.
* The `litex-term` implementation of SFL send [supports](https://github.com/enjoy-digital/litex/blob/994e422d1bf3cff985490341a86c16ad90750456/litex/tools/litex_term.py#L465)
  sending and waiting for ACK for up to 10 packets at a time. Right now,
  `teraterm-litex` waits for each packet sent to be acknowledge before sending
  the next one ([lockstep](https://datatracker.ietf.org/doc/html/rfc7440)).
* In the context of this plugin, I interchangeably call a packet a "chunk",
  and should probably be consistent.

## Acknowledgements
This software was made possible thanks to:

* The Tera Term authors, for creating a terminal emulator optimized for
  serial ports that I actually like!
* [RisohEditor](https://github.com/katahiromz/RisohEditor) so that I could
  create the LiteX dialog.
* @peddamat's article on [How to create a Rust DLL](https://samrambles.com/guides/window-hacking-with-rust/creating-a-dll-with-rust/index.html).
* @enjoy-digital for his tireless work on LiteX!
* The developers of [seq2gif](https://github.com/saitoha/seq2gif), for a quick
  way to demo my plugin.

## Footnotes
[^1]: Such scenarios include:
      * Creating a hand-tailored LiteX SoC
      * Running [Linux-on-LiteX](https://github.com/litex-hub/linux-on-litex-vexriscv),
      * Using a SFL protocol implementation besides the LiteX BIOS.

[^2]: As long as you're not compiling your firmware while the transfer is taking
      place. But why would you do that :)?

[^3]: I don't know where these strings came from; they seem to be handpicked
      random strings. They don't appear to be `mcookie`-based, which was my
      first guess:

      ```
      $ mcookie
      1c123a213bdaa17fb8c6d19ce2418e96
      ```

[^4]: `--target` alone passed to `cargo` is _not_ sufficient. You must invoke
     the 32-bit Rust compiler binary via `rustup`. Something like:

     ```
     rustup toolchain install stable-i686-msvc
     cargo +stable-i686-msvc build --release --target i686-pc-windows-msvc
     ```
