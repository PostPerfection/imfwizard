# Carried schemas

The XSDs `validate --xsd` checks the CPL, PKL, AssetMap and OPL against when no `--schema-dir` or `IMF_SCHEMA_DIR` is given. `xsd_validate.rs` embeds them with `include_str!` and writes them to a temp directory for xmllint. Each file is byte for byte what the URL below served on 2026-09-28, apart from the one fix to the PKL schema described at the end. The sha256 column is of the file as served. The directories follow the namespace paths the SMPTE registry lists them under.

The registry files import their dependencies by namespace alone, with no `schemaLocation`. `xsd_validate.rs` writes a driver schema that imports every namespace from its local file, so xmllint runs with `--nonet`.

The CPL namespace `http://www.smpte-ra.org/schemas/2067-3/2016` is the only CPL schema the registry publishes. Its namespace page names both ST 2067-3:2016 and ST 2067-3:2020 as specifying it.

| File | Namespace page | Downloaded from | sha256 |
| --- | --- | --- | --- |
| `2067-3/2016/st2067-3a-2016.xsd` | https://smpte-ra.org/schemas/2067-3/2016 | https://smpte-ra.org/sites/default/files/st2067-3a-2016.xsd | `a6d7b4221b025c02739936edd8e1bef39f609d1739ceb8632491aeb11274a719` |
| `2067-2/2016/PKL/st2067-2b-2016.xsd` | https://smpte-ra.org/schemas/2067-2/2016/PKL | https://smpte-ra.org/sites/default/files/st2067-2b-2016.xsd | `d9938a542f00d6b872781c9682b81f1ad9b21d00fdc8771a275e58bc9f099eb5` |
| `429-9/2007/AM/st-429-9-2014.xsd` | https://smpte-ra.org/schemas/429-9/2007/AM | https://smpte-ra.org/sites/default/files/st-429-9-2014.xsd | `cb75430ef32a52e8c4a8d98097cb267789be1b496e3518a00aebf3fe39cbc5dd` |
| `2067-100/2014/st2067-100a-2014.xsd` | https://smpte-ra.org/schemas/2067-100/2014 | https://smpte-ra.org/sites/default/files/st2067-100a-2014.xsd | `a35f3b057ca255757d2e1927c45c5c3b5c48837622bded5d5bbfec16f225d23b` |
| `433/2008/dcmlTypes/st433b-2008-am1-2011.xsd` | https://smpte-ra.org/schemas/433/2008/dcmlTypes | https://smpte-ra.org/sites/default/files/st433b-2008-am1-2011.xsd | `858b0f2e3d4677bf1a60448bc42df000cca33ce7af2345fe64814f265e471db3` |
| `w3c/2000/09/xmldsig/xmldsig-core-schema.xsd` | | https://www.w3.org/TR/xmldsig-core/xmldsig-core-schema.xsd | `d102ad3df7664c307e0c2c776ba4a90513b1969974d8a940bae1a77f9f21e15d` |
| `w3c/XML/1998/namespace/xml.xsd` | | https://www.w3.org/2001/xml.xsd | `61960fb3131e38022caad5360e2f33a3382578ab3c80cd58bd74320ede61b20c` |

The CPL and OPL schemas import dcmlTypes and xmldsig. The PKL imports xmldsig only. dcmlTypes imports xmldsig and the XML namespace. The AssetMap, xmldsig and XML namespace schemas import nothing.

## Licences

The SMPTE registry index at https://smpte-ra.org/ns states the terms for every SMPTE file here. The four SMPTE ST 2067 files also carry this text in their header, `st-429-9-2014.xsd` and `st433b-2008-am1-2011.xsd` carry none.

> Unless specified otherwise, XML elements are provided subject to the following license:
>
> Copyright (c), Society of Motion Pictures and Television Engineers. All rights reserved.
>
> Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:
>
> 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
> 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
> 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

The namespace pages for 429-9, 433 and 2067-100 repeat that notice and add:

> WARNING: The content of this page and the URLs listed below can change without warning and are not intended to be frequently accessed. Implementations must not (a) rely on the availability of this page and its contents and (b) access this page and its contents without user intervention.

`xmldsig-core-schema.xsd` states in its header that it "is governed by the W3C Software License", http://www.w3.org/Consortium/Legal/copyright-software-19980720, whose full notice reads:

> W3C(R) SOFTWARE NOTICE AND LICENSE
>
> Copyright (c) 1994-2002 World Wide Web Consortium, (Massachusetts Institute of Technology, Institut National de Recherche en Informatique et en Automatique, Keio University). All Rights Reserved. http://www.w3.org/Consortium/Legal/
>
> This W3C work (including software, documents, or other related items) is being provided by the copyright holders under the following license. By obtaining, using and/or copying this work, you (the licensee) agree that you have read, understood, and will comply with the following terms and conditions:
>
> Permission to use, copy, modify, and distribute this software and its documentation, with or without modification, for any purpose and without fee or royalty is hereby granted, provided that you include the following on ALL copies of the software and documentation or portions thereof, including modifications, that you make:
>
> 1. The full text of this NOTICE in a location viewable to users of the redistributed or derivative work.
> 2. Any pre-existing intellectual property disclaimers, notices, or terms and conditions. If none exist, a short notice of the following form (hypertext is preferred, text is permitted) should be used within the body of any redistributed or derivative code: "Copyright (c) [$date-of-software] World Wide Web Consortium, (Massachusetts Institute of Technology, Institut National de Recherche en Informatique et en Automatique, Keio University). All Rights Reserved. http://www.w3.org/Consortium/Legal/"
> 3. Notice of any changes or modifications to the W3C files, including the date changes were made. (We recommend you provide URIs to the location from which the code is derived.)
>
> THIS SOFTWARE AND DOCUMENTATION IS PROVIDED "AS IS," AND COPYRIGHT HOLDERS MAKE NO REPRESENTATIONS OR WARRANTIES, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO, WARRANTIES OF MERCHANTABILITY OR FITNESS FOR ANY PARTICULAR PURPOSE OR THAT THE USE OF THE SOFTWARE OR DOCUMENTATION WILL NOT INFRINGE ANY THIRD PARTY PATENTS, COPYRIGHTS, TRADEMARKS OR OTHER RIGHTS.
>
> COPYRIGHT HOLDERS WILL NOT BE LIABLE FOR ANY DIRECT, INDIRECT, SPECIAL OR CONSEQUENTIAL DAMAGES ARISING OUT OF ANY USE OF THE SOFTWARE OR DOCUMENTATION.
>
> The name and trademarks of copyright holders may NOT be used in advertising or publicity pertaining to the software without specific, written prior permission. Title to copyright in this software and any associated documentation will at all times remain with copyright holders.

`xml.xsd` carries no licence text. W3C's intellectual rights page, https://www.w3.org/copyright/intellectual-rights/, says "The W3C software license governs the reuse and modification of W3C software".

## Modification

`st2067-2b-2016.xsd` as published wraps the `UUIDType` pattern across a line break after the last `-`. XML attribute normalisation turns that CRLF into a space, so the pattern expects `- ` before the final twelve hex digits and xmllint rejects every PKL `Id`. On 2026-09-28 the two lines were joined into `urn:uuid:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}`, the pattern dcmlTypes and the AssetMap schema use. The sha256 of the fixed file is `ae572babd740b1f0a71bf5314a9587cea424e3a69b4b620516c28575b62f6778`. No other file has been modified.
