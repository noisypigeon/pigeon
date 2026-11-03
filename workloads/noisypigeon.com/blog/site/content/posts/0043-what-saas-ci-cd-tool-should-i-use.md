+++
title = "What SaaS CI/CD tool should I use?"
date = 2021-06-27T19:00:00+00:00
slug = "what-saas-ci-cd-tool-should-i-use"
description = "Musings about continuous integration and delivery tools."

[taxonomies]
tags = ["software-engineering"]
+++


Choosing a continuous integration platform for your team is like pigeons at the park. They’re all interesting but you don’t know which one you’re going to take home.

Here are the things I consider when choosing a CI system for a new project.

How many engineers will rely on this system?

I’ve helped customers whose entire engineering organizations would screech to a halt when their CI broke. This was often because the person who designed the pipeline left the company or was on vacation. For example: nobody knew how to publish a release manually or fix common CI problems like rebuilding a cache. Everything you automate should be accompanied by a guide for manual intervention (with screenshots!).

CI should be trustworthy and reliable but flexible enough that it can be fixed by someone without domain-specific CI knowledge, especially in a pinch. Write documentation and put comments in your config to help the next farmer in the CI pumpkin patch. You’ll thank yourself later when you’re not interrupted during your vacation!

In addition to this, consider what the engineering culture is like. Do people test their code frequently or at the end of the day? How often is code deployed to staging and production? This may change your decision on whether you should have faster builds or more concurrency.

What test tools does your project use?

While interviewing for my current job, I was tasked with implementing CI for a React Native project that used Detox (an E2E framework) and I chose CircleCI. Having just left Bitrise, my interviewers asked me why I didn’t chose that platform for this project. As much as I love Bitrise, as of then, Bitrise and Detox didn’t mix well. While I was working there, we’d sometimes get it to work for customers but it was a painful experience every time. I chose CircleCI for this exercise because of its first class Detox support. My colleagues at Circle had been working on Detox support for years before it was popular. Choose the provider with the best support for the tools you use!

There are a lot of CI tools but to cover my favourites:

-   CircleCI is amazing at Docker (containerization) and Detox.
    
-   Bitrise is top tier for release automation and support for official tools.
    
-   BuildKite is thee choice for on-site or bring your own infra solutions.
    
-   GitHub Actions has the best caching mechanism.
    
-   Honk CI is the best at stealing gardener’s hats.
    

Do you build on Mac, Linux or Windows?

As a general rule follow these guidelines:

1.  Native iOS and Mac projects need Mac build machines but as enticing as it is: don’t run Hackintoshes for CI. It’s against Apple’s EULA and genuine hardware is worth the extra cash. If your project is web-based or uses a hybrid framework such as React Native or Flutter, you have more flexibility in what operating systems and machines you can use.
    
2.  Unless you’re writing software for Windows, don’t use Windows for your build machines. It’s only pain. Having worked at CI companies, I was on the inside and watched my colleagues work on the different platforms. My colleagues, by far, complained about Windows (as a CI OS) the most. Please, for the love of god, don’t email me asking for Windows support!
    
3.  Use Linux where you can, Mac where you must. Mac build machines are expensive so I’ve seen companies use Ubuntu machines to run their React Native E2E tests and Macs for when they’re producing the iOS binaries and submitting to the App Store.
    

You should also consider if you want to go with managed or unmanaged infrastructure. I’m a bit of a masochist so I personally prefer unmanaged but it really depends on your needs and budget. Platforms like CircleCI and Bitrise primarily offer what is called “Cloud CI” which is managed infrastructure where they build off Amazon or Google and provide nicely packaged Virtual Machines. These machines are shared and you are only one of many tenants on the machine. VMs are created and destroyed as fast as a build start and finish.

For example, a 6-core 64GB Mac Mini could be cut up as much as 12 VMs providing each tenant half of a CPU core and just over 5GBs of memory. Performance generally sucks and you spend a lot of time optimizing by using caching, skipping unnecessary steps and so on. The benefit to managed hardware is the easy breezy experience. If Amazon has a problem it’s Circle’s engineers working on a fix, not you.

How much do you have to spend?

Cost is something that we, as individual contributors, generally don’t have to think about. For example, and only if you want to be horrified, do the napkin math for what your company is paying Slack. With that said, as I’ve transitioned into the role of a systems architect, I’ve learned to choose solutions as if it was going to be my own credit card on the invoice. It can be enticing to get the best of the best when the good of the good would do. For example, maybe longer builds is tolerable when more builds can run concurrently. And even if you’re going to go with the best of the best, consider if it’s a good value or if something that is 20% less money for 5% less performance would suffice.

Time is also a cost to consider. Implementing a trustworthy and robust CI system frees up a lot of time because individual dev machines aren’t bogged down running E2E tests or code signing binaries allowing engineers to continue their important work. Watching pigeon videos on YouTube!

C-Levels, in my experience, like getting their money’s worth and paying for CI to increase developer efficiency is usually a worthwhile cost. If you’re looking for ways to help your proposal and convince your CEO, I’ll be writing a blog post on this topic soon. 🥺👉🏻👈🏻 CEO can we have CI pwease.

I personally love credit-based billing, which a lot of the platforms have started to support. It allows you to use Mac machines only when necessary and have more powerful Linux-based machines (meaning faster builds) for regular tasks.

I hope that you learned something or this blog post helps you in the journey of choosing your next CI solution.
