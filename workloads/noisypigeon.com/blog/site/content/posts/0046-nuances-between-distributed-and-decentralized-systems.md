+++
title = "Nuances between distributed and decentralized systems"
date = 2024-03-07T08:00:00+00:00
slug = "nuances-between-distributed-and-decentralized-systems"
description = "Essay about the shortcomings of decentralized systems in the context of DeFi/Web3 systems."

[taxonomies]
tags = ["software-engineering"]
+++


Over the past 4 years, I’ve worked both as an infrastructure and back-end engineer designing and building distributed systems for Web3 ecosystem projects. But working in an industry that loves to use the moniker “decentralization” - how do distributed systems differ from decentralized systems, and why are a lot of Web3 ecosystems, protocols, and dApps (decentralized apps) actually mostly rather centralized?

**All decentralized systems are distributed systems but not all distributed systems are decentralized systems.** This blog post goes into the nuances between distributed and decentralized systems as well as some tangible things to consider to foster decentralization within your infrastructure, tooling, and company.

**Firstly, I’d like to define these terms for the purpose of this blog post:**

-   Distributed systems: Composed of multiple separate computers that communicate and coordinate with each other to manage a common workload offering advantages including increased resilience against single points of failure, fault tolerance, high-availability, and opportunity for scalability but controlled or governed by a centralized authority.
    
-   Decentralized systems: Characterized by the absence of a single central authority or point of control; distributing decision making authority and data across a network of interconnected nodes and network participants offering several additional advantages including further increased resilience against batched points of failure, censorship resistance, and multi-operator scaling.
    

The surface area for “distributed systems” is a lot more inclusive and manifests in multiple forms and levels of complexity because the primary focus is reducing single points of failure and increasing system resiliency and fault-tolerance. Decentralization on the other hand has a rather rigid definition that can be very difficult to effectively accomplish in practice.

**A. What is an example of a static centralized deployment?**

This could look a lot of different ways but a pattern I’ve seen is that each service is on it’s own EC2 instance (i.e.: 1 EC2 instance for front-end, 1 EC2 instance for back-end) and a RDS instance for the database. This resource architecture demonstrates the benefits of vertical scaling since as traffic and demand increases, the infrastructure/DevOps folks can manually increase the compute, memory and disk capacity available to the front-end, back-end, and database. But vertical scaling can go so far as single instances requiring higher resource classes that cost more money. Additionally single deployments will always pose inherent risk such as requests being dropped/not fulfilled due to demand being too high or when the service is unavailable due to the downtime between resource class changes and restarts. 

Anecdotally, a lot of Web3 “dApps” use this deployment strategy even in production with high activity.

**B. How can this be improved with “low effort” distributed centralized architecture?**

The single EC2 instances could be converted to use EC2 auto scaling groups which utilize an ELB (elastic load balancer) to stabilize incoming traffic between instances and configure rules to create new instances as capacity is exhausted. Depending on the specific requirements and usage patterns of the database, the single RDS instance could be replaced with a clustered RDS instance (such as multi-AZ or Aurora) to provide (for example) different nodes for write vs read operations, or a primary node with sharding and/or logical duplication for follower/accessory nodes. 

These two adjustments provide a massive amount of scaling headroom because policies can be configured to automatically increase and decrease available resources on a per instance basis as well as increase or decrease the number of instances being created.

**C. How can this be improved with a “mid effort” distributed centralized architecture?**

I’d personally say that a lot of projects/companies could go forever using auto scaling groups and compute platforms like EC2, and even go as far as saying a lot of services using container orchestration platforms don’t inherently need them to operate and scale effectively. With that said, the next iteration would be to switch from EC2 to ECS. ECS is a great mid-step between the next section of what I’d categorize as “high effort” distributed centralized architectures as it abstracts most of the considerations that need to be intentionally thought out/designed with Kubernetes or Nomad. 

For some context, AWS has put a lot of work into the EC2 platform so instances behave like containers for the most part but they are actually virtual machines running on bare metal. It’s out of scope of this article so I recommend reading this [Google Cloud explainer](https://cloud.google.com/discover/containers-vs-vms) for more details and I’ve added a footnote \[1\] for my high-level overview/summary of the differences.

The front-end and back-end running on the individual EC2 instances (or EC2 auto scaling groups) could be containerized and defined as tasks to be orchestrated as long-running tasks called services. These tasks and services can be configured to expand/contract resources similar to EC2 auto scaling groups however they have a considerably lower latency to service discovery and operational readiness. compared to a virtual machine’s “cold start” / boot process.

In the effort of staying on topic, effective monitoring and observability will be an entirely different blog post but container orchestrator platforms make monitoring and observability a lot easier to do effectively due to containers being on the same data plane allowing for more effective inter-service communication.

**D. What does a “high effort” distributed centralized architecture look like?**

From cloud vendor lock-in container orchestration platforms like AWS ECS and Google Cloud Run Jobs, Kubernetes and HashiCorp Nomad are both very portable meaning they can be deployed on all public clouds like AWS, GCP or Azure, as well as locally or in a collocated environment. 

This basically opens up Pandora's Box in terms of possibilities for scaling and adaptability, and provides a pathway to cloud native decentralized architecture. 

Both Kubernetes and HashiCorp Nomad are popular platforms for orchestrating workloads, but they differ in design philosophies and ownership. While Kubernetes is widely adopted and favored for its open-source nature and strong industry support from major cloud providers like AWS, GCP, and Azure, Nomad offers a simpler, lightweight solution with support for various environments, including containers, VMs, and executables. However, in my mind and for the context of this article, Nomad's ownership by a private corporation raises concerns with regards to vendor lock-in and centralized authority. For the rest of this article, I’ll focus on Kubernetes for its exhaustive open-source/foundation driven ecosystem, community support, and integration with open-source tools like Helm and Prometheus which make it my and many others preferred choice.

This level of flexibility and versatility provides so much in terms of high-availability workloads and additional flexibility scaling beyond multi-AZ (availability zones) and multi-regional (physical data centers) because it provides a cloud native pathway to multi-cloud deployments using peer-to-peer network cluster gossiping for example (although I’ve observed/read other methods as well).

**E. How can I make a “high effort” centralized distributed architecture decentralized?**

Finally, to the nuances of distributed vs decentralized. A distributed system is defined as a system with minimal to no single points of failure but a decentralized system is defined as one with no single points of authority or control. There’s a lot about the philosophy of decentralization and decentralized ownership which I think is overlooked by the majority of the Web3 and DeFi community. 

Decentralization is not about the individual system but how a distributed system fits into the greater world and what influences can affect its performance and viable operation when pressure is added by corporate interests, government censorship, bad actors and collective usage. For a system to be truly decentralized, it should incorporate robust mitigations for:

**E.a. Corporate (Cloud) and Government Censorship Resistance**

([E.A. sports, it’s in the game.](https://www.youtube.com/watch?v=SgGK93iiQ6Ehttps://www.youtube.com/watch?v=SgGK93iiQ6E))

Decentralized systems such as Ethereum and Bitcoin are only trustworthy and function because they contain a distributed ledger that thousands of nodes observe, validate new transactions and seed copies of the archives to be referenced again and again. The consensus and re-org process ensures that variations of data are merged and generally once a block is finalized, compounded by the weight of newer blocks being finalized with the previous one as the foundation, we can trust that the data is authentic.

With that said, there are concerning data points that chip at even the gold standard decentralized systems like Ethereum where 52% of Ethereum nodes run on AWS, followed by 16.9% on Hetzner (source: [CoinTelegraph](https://cointelegraph.com/news/3-cloud-providers-accounting-for-over-two-thirds-of-ethereum-nodes-data)). In that 52% of nodes on AWS, 24% of nodes are in a single region: us-east-1 (source: [EtherNodes](https://www.ethernodes.org/country/United%20States)). This means that AWS (or the U.S. government) could outlaw Ethereum nodes on their platform and overnight 52% of all nodes securing Ethereum could disappear overnight enabling a number of attack vectors such as a 51% attack, Sybil attack, and more that could irreversibly compromise the trust and verifiable authenticity of the network and it’s participants.

If decentralization is the goal, we should strive to not be bound by a single public cloud provider or of that public cloud provider in a single geographical location that is overseen by a single government. More on this with:

**E.b. Natural Disaster and Geographic / Political Distress and Conflict**

Another area of decentralization is mitigating the risk of data loss due to natural disasters or political distress and conflicts focused in a certain geography. GCP has 3 US regions and 2 Canadian regions on the east half of North America in South Carolina, North Virginia, Ohio, Ontario, and Quebec. AWS has 2 major US regions and 1 Canadian region on the east half of North America in North Virginia, Ohio, and Quebec. This is an important consideration for my first point however is more evident with a natural disaster scenario. If a deployment was decentralized on the basis of a public cloud provider alone but a single region such as gcp-us-east4 and aws-us-east1 both being in North Virginia, a natural disaster or political distress/conflict affecting public infrastructure could result in the region going offline.

**E.c. Attacks on Ownership through Denial of Resources** 

This is an attack surface that is becoming more prominent these days. We are used to mitigating for denial of service attacks where a bad actor attempts to overload a service to cause it to go offline however a more recent method of attack is to cost the owner of the service as much money for cloud resources and invoking costly resources (such as push notifications, SMS codes, GPU powered machine learning backed CP/hate symbol detection algorithms, photo ID verification systems) where a company is paying for that interaction regardless of the outcome of the interaction. This is something that requires robust abuse prevention, rate limiting, and detection/notification for human intervention.

Decentralized systems, especially ones that may be funded by a centralized entity, should be operated by multiple parties with separate accounts and payment methods/fund buffers. This mitigates a lot of risk and makes it a lot more difficult to successfully exploit a denial of resource attack that results in a system going offline in whole.

**F. In summary**

I don’t believe most projects need to be decentralized and I wish/hope that the Web3 and DeFi ecosystem and community would transition away from the decentralized vocabulary or put in the work to ensure the ecosystem is truly mitigated from government and corporate censorship, damage to physical infrastructure due to natural disasters and conflict (war, political, etc) focused on geographical locations, and attacks on ownership such as denial of resource attacks. I have worked in a couple roles where these considerations were worked towards especially with balancing and distributing our workloads beyond the traditional two availability zones/regions but several between multiple public clouds and accounts.

Footnotes:

\[1\] A high level overview is that VMs carry involuntary constraints that affect cost and operational vitality due to overhead of the hypervisor and that they run a full operating system with dedicated resources (CPU compute, memory, storage, networking, sometimes GPU compute). A container provides a much lighter resource footprint that encourages more “ephemeral-like” usage patterns because they’re faster to create and enable higher density within the parent resource.
