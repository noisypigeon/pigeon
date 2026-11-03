+++
title = "PSPDFKit for Web 2019.5 Release"
date = 2019-09-09T07:00:00+00:00
slug = "pspdfkit-for-web-20195-release"
description = "Mirror of PSPDFKit for Web 2019.5 Release notes."

[taxonomies]
tags = ["software-engineering", "career"]
+++


> 📖 **Note**: This post is from the archives. It may reflect outdated views or a [previous name](/pages/names/). This is a mirror of blog post I co-authored for the [PSPDFKit for Web 2019.5](https://pspdfkit.com/blog/2019/pspdfkit-web-2019-5/) release with Philipp Spiess, published September 9th, 2019.

We are pleased to announce PSPDFKit for Web 2019.5! This release introduces our new Form Designer component, supports embedded JavaScript in PDF files on Standalone, and is distributed via Docker Hub and npm. Please refer to our Server and Web changelogs for a complete list of features and bug fixes.

**Form Designer**

We are excited to introduce Form Designer, a new component for PSPDFKit for Web. With Form Designer, you can now build workflows in which your end users can create and edit PDF form elements. In fact, one of our customers already successfully implemented a new signing workflow using Form Designer. Their use case involves one user (the “sender”) preparing a document for signing by adding signature fields and sending it out to one or many “signer(s).”

The Form Designer component enables an extensive set of APIs for creating and editing various PDF widget types — including text fields, check boxes, and signatures — and is available for both Standalone and Server-backed deployments.

Please email us at REDACTED if you are interested in developing on top of our Form Designer component. We’d love to hear about your use case and talk about how you can best leverage Form Designer for it.

**JavaScript for Standalone Deployments**

Some interactive PDF forms use embedded JavaScript to add custom validation rules, calculations, or other interactive functionality. We’re excited to bring support for these JavaScript features to PSPDFKit for Web Standalone.

We are using a custom, sandboxed JavaScript VM and do not run the embedded JavaScript in the browser directly. Nevertheless, JavaScript embedded in a PDF — much like macros in Microsoft Word files — can lead to behavior that’s surprising and unexpected for users. This is why we decided to let customers choose to opt into executing JavaScript in PDF files. Learn more in the new JavaScript guide article about how to enable support. Support for Server-backed deployments is coming in the future, so if you are interested in this, please email us at REDACTED.

**Docker Hub and npm**

To make it easier to integrate PSPDFKit for Web into your existing infrastructure, we’re thrilled to announce that we now publish PSPDFKit on Docker Hub and npm. Existing distribution channels are not affected by this change and will still be supported.

Along with all the new features, this release also includes numerous bug fixes and minor improvements. For a complete list of changes, see the PSPDFKit for Web 2019.5 and PSPDFKit 2019.5 Server changelogs.
