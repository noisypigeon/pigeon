+++
title = "WX 5100 + GTX 1070 Ti (Dual GPU Workstation!)"
date = 2018-08-04T19:00:00+00:00
slug = "wx-5100-gtx-1070-ti-dual-gpu-workstation"
description = "Mirror of a build list published on PCPartPicker on August 4th, 2018."

[taxonomies]
tags = ["hardware-and-builds"]

[extra]
og_image = "/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-34b6d333329c97f703a4aa443fe64926.jpg"
+++


> 📖 **Note**: This post is from the archives. It may reflect outdated views or a [previous name](/pages/names/). This post was initially published on [PCPartPicker](https://pcpartpicker.com/b/fjsZxr) on August 4th, 2018.

<figure class="attachment attachment--preview attachment--jpg"><img height="1066" width="1600" alt="An image with caption: Wide shot of earlier desk." loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-5bcf4d3ff055fee62063c5299efd0f0d.jpg"><figcaption class="attachment__caption" aria-hidden="true">Wide shot of earlier desk.</figcaption></figure>

This is revision two of my build. I have been building custom PCs and fixing up OEM PCs since I was a kid however I only had the opportunity to build them for other people. And since I worked as a freelancer for many years (often having to move from one city to another for work) a desktop with monitors, peripherals, etc was unfeasible at the time. Fortunately, I recently had the opportunity to settle down and move into a permanent place and decided it was time to treat myself to a custom built PC.

This workstation serves two purposes. Firstly, this computer is my daily driver for my full-time job as a Support Engineer. I work at an organization that builds a set of tools that help developers automate the process of testing their code so they can detect and fix problems earlier. My job entails helping other software engineers work through problems with our product by creating test projects, configs, and writing scripts, running customer projects in isolated environments (generally remotely), and answering support tickets through our CRM tool.

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="1600" alt="An image with filename: 231087.37a13f52d3a3cb2e983b4258b077cfef.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-37a13f52d3a3cb2e983b4258b077cfef.jpg"></figure>

However, this computer isn't only for work, which leads me into its second purpose: my hobbies. This computer enables me to make better 3D CAD drawings in Fusion 360 and FreeCAD, music production in Ableton Live and FL Studio, and game development (although I'm still figuring out my workflow). I also use this machine to play games such as Stardew Valley, Overwatch, Metro Last Light, and GTA V.

This workstation is configured to have a host and a guest. The host uses my Linux distro of choice - Fedora 28 which I have customized to my liking, and the guest uses Windows 10 Pro. The host is running on the metal while Windows 10 runs in a virtual machine. This is configured using KVM, QEMU, and VFIO/VirtIO passthrough (VFIO for PCI devices, VirtIO for networking and peripherals). Fedora owns the 250GB NVMe SSD and WX 5100 graphics card while Windows 10 Pro owns the 1TB NVMe SSD and the GTX 1070 Ti. Each operating system has access to 4-Cores, 8-Threads and 16GBs of memory.

Peripheral management is currently handled by VirtIO/USB Host Device passthrough however I'm currently setting up evdev for easier peripheral management. I access the operating systems through one Dell UltraSharp 27" LED Monitor (model number U2715H) however, I plan on adding a second monitor or a hardware KVM switch in the near future (since switching display inputs is still a pain).

## Pictures

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="900" alt="An image with filename: 231087.34b6d333329c97f703a4aa443fe64926.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-34b6d333329c97f703a4aa443fe64926.jpg"></figure>

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="900" alt="An image with filename: 231087.a585bea1766ce763289946dd42251ace.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-a585bea1766ce763289946dd42251ace.jpg"></figure>

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="1600" alt="An image with filename: 231087.92688828b36b7b2c7d864a78bbe5d01e.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-92688828b36b7b2c7d864a78bbe5d01e.jpg"></figure>

<figure class="attachment attachment--preview attachment--jpg"><img height="1066" width="1600" alt="An image with filename: 231087.90d37129a3ecb8bda7905f0441b0fc74.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-90d37129a3ecb8bda7905f0441b0fc74.jpg"></figure>

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="841" alt="An image with filename: 231087.f4bf6a9281ad49dd62f36b0fa0ed2f56.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-f4bf6a9281ad49dd62f36b0fa0ed2f56.jpg"></figure>

<figure class="attachment attachment--preview attachment--jpg"><img height="1200" width="900" alt="An image with filename: 231087.fea5dced1404e067aa279a51cd32a8e7.1600.jpg" loading="lazy" src="/assets/images/posts/wx-5100-gtx-1070-ti-dual-gpu-workstation/231087-fea5dced1404e067aa279a51cd32a8e7.jpg"></figure>

Here are some comments about the components I chose for my build: 

AMD Ryzen R7 2700X CPU

This build has been a work in progress and I have been planning this build since August of 2017. I put together a lot of different parts lists that eventually got scrapped for one reason or another. I started buying parts for Revision One of this system in early May 2018 and if Pinnacle Ridge wasn't released when it was, this build 100% would've used a 8700K. First generation Ryzen was enticing but I needed a computer I could rely on and I saw/read/watched mixed reviews about this. However, when Pinnacle Ridge dropped, I was very impressed by the improvements Zen+ brought to Ryzen's overall system stability and thought I'd take a chance. Another thing that contributed to my decision was that AMD announced their AM4 platform would be supported for longer than a couple generations, allowing me to get one last big upgrade before AM5 comes out. I also picked my parts with Intel compatibility in mind in case Ryzen was not stable enough. With that said, after almost three months of using Ryzen every day, I am very happy with the stability and plan to keep Ryzen/AM4 as my platform for the next 4-5 years. I have also overclocked this CPU to 3.9GHz which has been mostly stable for me.

Asus ROG Crosshair VII Hero X470 Motherboard

I ultimately chose the C7H because of der8auer's and buildzoid's coverage. I needed a motherboard that will last multiple years and will likely support new Ryzen CPUs (i.e 3700X, 4700X, or 5700X). I also liked that this motherboard has eight USB 3 Type A ports since I would've had to use a USB 3 dock with other options because of all of my peripherals. I also removed the plastic "heatsink" over the I/O as I personally think the motherboard looks cleaner without it. I opted for the non Wi-Fi/Bluetooth model because I read from others in the community that the Wi-Fi model wasn't as stable as it's non Wi-Fi enabled sibling. While some will see this as a compromise, my desktop sits beside my gigabit router plugged in with a cat6 cable giving me between 800 to 1100mbps down and 200 to 400mbps up. If I ever need to move to a place where this is no longer true, I can buy a dongle or a PCIe add-in card. However, I'd rather run a 15 foot ethernet cable across my house then use Wi-Fi as it severely bottlenecks my speeds.

G.Skill Trident Z RGB 32GB DDR4-3200 CL14 14-14-14-34 Memory

This memory kit is awesome however if I'm being honest, I would've preferred a non-RGB kit but it was the only Samsung b-die kit with rated 14-14-14-34 timings that I could find in stock at the time. I opted for 32GBs so both my host and guest could have access to a full 16 gigabytes of that glorious high-speed memory _chef kiss_. This kit is currently overclocked to 3266MHz at those timings.

Be Quiet! Dark Rock Pro 4 CPU Cooler

This cooler is massive - some folks do not like this but I do. I removed the outer fan to allow for better visibility of the memory DIMMs and my temperatures didn't budge. My 2700X still runs in the high 50s, lower 60s when under full load (while running at 20-30s when idle). And yes, I could've used the stock cooler but I wasn't a fan of the RGB. I was able to quickly sell it on the second hand market.

Samsung 970 Evo 250GB and 970 Evo 1TB NVMe SSDs

As mentioned, the 250GB SSD is for Fedora and my files from work. The 1TB SSD is for my Windows 10 virtual machine which stores my games, music production projects, and personal files and documents. The drives are blazing fast and I love how M.2 doesn't require any additional/unnecessary cables.

AMD Radeon Pro WX 5100 and NVIDIA GeForce GTX 1070 Ti Graphics Cards

The WX 5100 is for Fedora and 3D CAD while the GTX 1070 Ti is for Windows via GPU passthrough for gaming. I am currently using a 27” 1440P 60Hz monitor from Dell and I’m getting over 60 FPS in all of my favourite games (while in the virtual machine). There are still some tweaks/optimizations I have to do but I am overall very happy with the performance.

EVGA SuperNOVA G3 750W Power Supply

I needed a good, reliable power supply for my build and this model was recommended to me by multiple people in the community. I purchased the 750W variant for additional leg room/future upgrades as well as to provide me with enough power for my overclocked Ryzen CPU, and two graphics cards (the 1070 Ti being overclocked as well). It has worked perfectly for my needs.

CableMod Pro series Nylon custom cables

They're CableMod's Pro series Nylon cables in Silver, Grey, and Pink. I purchased them in 700mm (24-Pin, PCIe), and 750mm (EPS) lengths to not limit my future options in terms of compatibility with other cases.

I also got a coupon with my C7H which arrived after I made my order with CableMod. I contacted their support team and they awesomely applied the coupon retroactively and refunded me the difference even though it was a few weeks after. Great customer support team! Would buy again. While some folks would say the cables aren't worth it because of their price, I think they are! I'm not a huge fan of RGB but still wanted a pop of colour in my build.

Fractal Designs Meshify C White TG ATX Case

I wanted a compact ATX enclosure that had good airflow and wasn't gaudy. I decided to go with the Meshify C because of the mesh front panel and small-ish form factor. I chose the white model because it fits better with my house. This case was a pleasure to build in and I have some case/paint mods planned for the future which I'll definitely update you all on once they happen.

Be Quiet! SilentWings 3 120mm PWM Case Fans

I have added three Silent Wings 3 fans around my chassis (two in the front as intake and one in the back for exhaust). There are also two additional Silent Wings 3 fans that came with my CPU cooler however those ones are 135mm. They are so quiet, it's honestly amazing. Under what I would call extreme load, the fans are only slightly audible. I would definitely recommend these to anyone who is noise sensitive like me.

Cyber Power CP1500PFCLCD 900W Uninterruptible Power Supply

I purchased this unit to provide emergency power in case of a power outage to provide me with enough time to safely shut down my machine. This unit provides enough power for my PC, monitor, router, and modem and it has helped me get through one power outage already! Very impressed.

I am very happy with my system's performance but I still have some tweaking to do. I'm still working on overclocking and benchmarking all of my components so I will update this section in the coming weeks once I've had the opportunity to do so. I will be using Cinebench, 3DMark, AIDA64, Geekbench 4, Metro Last Light, and GTA V to benchmark Windows 10, and I am still trying to determine what benchmarking tools I should use to test Fedora 28 (please send me your recommendations!).

However, as a general note, I am getting over 60fps in most titles while in the Windows VM using GPU passthrough with a 1440P panel and high or medium quality settings depending on the game. This section will be expanded soon.

This is my first custom build that I made for myself and it was a lot of fun planning and building this system. It's not final but Revision Two is so much closer to my vision. I'm so excited and grateful to have this sitting on my desk. Thanks to everyone who helped me on the Gamers Nexus, Level1Techs, Passthrough Post, Linus Tech Tips, and PCPartPicker forums and Discord servers. PS: I am still planning on using my old desk (see later photos), it's just currently being repaired.

Let me know what you think and please share constructive criticism! Additionally, **I would love to hear from you all about what I should name my build.** The name of the hen figurine is Bimsy but I'm not sure that fits the build haha.

Thanks for taking a look and happy building!
