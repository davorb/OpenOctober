# Contributing to the Project

Thank you for your interest in contributing. This document sets out the principles under which the project is developed and the practices we ask every contributor to follow. They keep the project independent, lawful, and sustainable.

## Nature of the project

This project is an independent implementation of a game engine, client, and server, written from scratch. It has its own architecture, its own source code, and its own network protocol. It does not contain, bundle, or distribute any original game code or assets.

The project is a new and separate program. It does not reproduce, adapt, or substitute for any original software. Its purpose is to let users who already hold a legitimate copy of the original game data run that data on modern systems, using independently written software. It is not a replacement for the original product and is not a means of obtaining it.

## Legal framework

Copyright protects the particular expression of a work, such as its source code, artwork, music, and text. Ideas, principles, functionality, algorithms, protocols, and data formats are not protected by copyright. This principle is recognized in Article 1(2) of the Software Directive (2009/24/EC) and in Swedish law.

The project is developed within the EU interoperability framework:

- **Directive 2009/24/EC, Article 6**, implemented in Sweden as **Upphovsrättslagen (1960:729), 26 h §**, allows a lawful user to carry out the analysis needed to obtain the information necessary to achieve interoperability between an independently created program and other software or data.
- **Article 5(3)** allows a lawful user to observe, study, and test the functioning of a program in order to understand the ideas and principles that underlie it.
- Under **Article 8**, these rights cannot be removed by contract.

The project relies on these provisions as follows:

- **The project's program is independently created.** Every line of project code is written by contributors from their own understanding of observed behavior and published format information.
- **Analysis serves interoperability.** Study is limited to the interfaces, file formats, and protocols needed for the project to work with user-supplied data.
- **Information is used only for that purpose.** What contributors learn is recorded as neutral technical description. It is used to enable interoperability and for nothing else.
- **The resulting expression is the project's own.** Structure, naming, organization, and design are original. The project's code is not a copy or adaptation of the original code, and it does not seek to reproduce its form.

## Core principles

1. **Interoperability only.** Analysis goes no further than what is needed to understand the interfaces, formats, and protocols required for interoperability.
2. **Independent implementation.** Contributors implement behavior in their own way, with their own design and their own code.
3. **User-supplied data.** The project never distributes original assets. Users provide data they have lawfully obtained.
4. **Documented provenance.** Contributors keep clear records of how their work was developed.

## Practices we ask of contributors

### Do

- Write your code yourself, based on your own understanding of observed behavior.
- Describe formats, protocols, and engine behavior in your own words, focusing on what they do rather than how any original code is written.
- State your sources in pull requests, for example "observed through black-box testing," "derived from public format documentation," or "based on community specification X."
- Use publicly available documentation, existing community format specifications, and your own testing as your first sources of information.
- Keep specifications, notes, and tests that show how your implementation was reached.
- Design the project's data structures, APIs, and architecture on their own merits.
- Test only with game data you have lawfully obtained.
- Let a maintainer know if you have had prior access to original source code (see "Clean-room practice").
- Ask a maintainer if you are unsure about anything in this document.

### Do not

- Do not copy, transcribe, or closely follow original source code in any form, including leaked, decompiled, or disassembled code.
- Do not contribute decompiler or disassembler output, or code that follows its structure, naming, or organization.
- Do not contribute material that was obtained in breach of a confidentiality obligation, or that was leaked or otherwise not publicly available.
- Do not add original game assets or content derived from them to the repository. This includes graphics, audio, text, maps, data files, and test fixtures.
- Do not link to or provide instructions for obtaining unlicensed copies of the original game or its data.
- Do not add tooling that circumvents online services or copy protection of running software, such as licence checks, anti-cheat, authentication of official servers, or access controls. Reading the data files of your own lawfully obtained copy, so that an independent program can work with them, is the interoperability this project exists for.
- Do not use the original game's names, logos, or branding in ways that suggest affiliation with or endorsement by its rights holders.
- Do not use information gained through analysis for purposes other than enabling interoperability.

## Guidelines for analysis

Where analysis of original software or data is needed to establish interoperability:

- You should be a lawful user, meaning someone who owns or is licensed to use the copy you analyze.
- Check first whether the information is already available, for example in existing documentation or community specifications, and use those sources where they suffice.
- Limit your analysis to the parts needed for interoperability, such as a file header layout, a record structure, or a message format.
- Record your findings as neutral technical descriptions of format and behavior.
- Keep decompiled or disassembled material out of the repository, issues, and pull requests.

## Clean-room practice

Where practical, we favor a two-stage approach:

1. Observed behavior and formats are written down as a plain-language specification.
2. A contributor then implements from that specification, working independently and without reference to the original code.

This separation makes it straightforward to show that the project's code is the product of independent development. If you have had access to original source code, for example through past employment, please tell the maintainers before contributing to the related area. We can then make sure the work is organized appropriately.

## Assets and data

- The project does not host, mirror, or distribute original game data.
- Users are responsible for lawfully obtaining their own copy of the data.
- The project's tools read user-supplied data in place. They are not intended to extract and redistribute it.
- Please avoid adding screenshots or excerpts of original content to the repository. If you think documentation needs them, check with a maintainer first.

## Contributor declaration

By submitting a contribution, you confirm that:

- the contribution is your own work, or you otherwise have the right to submit it under the project's license;
- it does not include code, assets, or other material taken from the original software or from any source you are not entitled to use;
- you have followed the practices described in this document.

Contributions are accepted under the license in the `LICENSE` file. We may ask that commits carry a `Signed-off-by:` line under the Developer Certificate of Origin.

## Raising concerns

If you believe a contribution may include material that does not meet these standards, please contact the maintainers privately. We will review it promptly and make any necessary changes.

## Scope of this document

This document describes the project's contribution practices. It is not legal advice, and the law varies between jurisdictions. If you have questions about your own circumstances, please consult a qualified professional.

Thank you for helping keep the project independent, careful, and respectful of the work that inspired it.
