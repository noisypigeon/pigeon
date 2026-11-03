+++
title = "Reconnecting Fragments: The Evolution of My Names through Email accounts"
date = 2025-07-29T07:00:00+00:00
slug = "reconnecting-fragments-the-evolution-of-my-names-through-email-accounts"
description = "Essay about reconnecting fragmented identities."

[taxonomies]
tags = ["identity-and-transition", "data-hoarding", "personal-essays"]
+++


> **Related Blog Posts** 📚
> 
> -   ↪️ 2022-08-25: “[_How to Write a Support Ticket_](/posts/how-to-write-a-support-ticket/)“
>     
> -   ↪️ 2016-01-01: “[_I’ve got something to share! (coming out as trans)_](/posts/ive-got-something-to-share-coming-out-as-trans/)“
>     

I have been more intentional about my online presence in recent years, especially how I want to build continuity between my [different names and aliases](/pages/names/). I’ve spent countless hours exploring Wayback Machine to recover old blog posts and career accomplishments. 

Something I’ve been asked many times over the years is Why So Many Names? I think this is a difficult question to fully capture but I want to explore this topic and how I’ve worked towards that goal through consolidating my email accounts and private records.

## **History**

When I [formally came out as trans](/posts/ive-got-something-to-share-coming-out-as-trans/) in January 2016, I was disowned and kicked out by my family. In addition to changing my first name to Willow, I processed the grief by changing my last name as well. This was a very destabilizing time for me and having nothing to tether me contributed to this. It took me some time to land on a last name that felt normal:

Being a minor, it was very difficult to get any stable form of employment or housing. I took Greyhound buses and budget airlines across the North America and Europe. From freelance work, short term contracts and MLH hackathon prices, I was constantly on the move.

I temporarily lived in Alberta which has a lower age of majority, allowing me to legally change my name without a guardian at 18 instead of 19. However, at the time I was navigating barriers and stalking that influenced me to change my first and middle name to something different. I was intimidated and unsure how to protect myself. When the waiting period of 180 days ended, I was able to legally change my name in Alberta. I changed my name to: Luna Oliver Graysen.

However, at this point, I had moved away from Alberta, lived in Berlin for several months and recently returned to Canada and was living in Montreal. I sometimes used Oliver and the French adaptation Olivier for safety.

My legal name stayed Luna Oliver Graysen for several years however my technical career evolved and changed quite drastically. I worked in support engineering, open source, and developer relations roles on the front lines where [aliases were a helpful tool to preserve my privacy](/posts/how-to-write-a-support-ticket/). I also participated at in-person events which unfortunately posed safety risks for me. This is where names like Bailey and Rhu come into the mix.

Using aliases at work started to spread into daily life and I fell out of love with the name Luna. I disliked Oliver because people assumed he/him pronouns out of the gate. I shifted to nicknames like Ollie and other variations but it was time for a change. As a Quebec resident, I legally changed my name to Kara Luna Graysen.

At the time I felt that it was giving homage to Luna while still allowing me to shed the name from daily use and I found Kara more suitable.

One critical mistake however came to haunt me a few years later. The names were too similar that when I updated my name with financial institutions and the CRA, the credit bureaus Trans Union and Equifax created a separate file in their systems. This was not caught for 3 years as certain financial institutions would attach credit accounts to Luna Graysen and some Kara Graysen obstructing my window into fraud and misuse of my credit.

When I separated with my ex and caught wind of my identity theft, I spent 9 months working with Equifax and TransUnion to consolidate my fragmented unique identities. While I am still uncovering everything and remain in the process of resolving all of the disputes, it was incredibly disheartening to see how poor the validation is of these systems. I would like to unpack what happened with my identity being stolen at some point but for now I’ll leave it at that.

After I had re-settled in Vancouver after moving from Montreal. I decided once again to legally change my name. However, the intent behind this was to sever the credit bureaus blended validation of my documents. I had discussed my situation with a lawyer, an insolvent trustee, and other advisory services, and the best way to prevent new credit lines from being open was consolidating my bureaus and legally changing my name to be distinct enough that old identity documents _should_ not pass validation. I landed on Kara Finch.

I learned that I do not legally need a middle name and decided to omit one entirely. This served the purpose that my credit is truly frozen and I uh… like finches (pigeon would be a weird last name, I’m sorry).

You’re probably asking why my legal name is Kara Finch if I have returned to Willow and did so around the same time. The answer is due to the complexities of my situation and ongoing legal processes to dispute fraudulent debt. A full name change could work against me. Which is why, once everything is settled and over in/around 2027, I will be legally changing my name to Willow Finch. Completing a full circle of name changes back to my original true name.

## Recovery

With all the lore aside, as I worked through Wayback Machine and other resources, I needed to recover these accounts. Each name:

-   D$&gl@s @l&x@nd%r B&mb#.
    
-   Willow Alexandra Bellemore.
    
-   Willow Rose Belle.
    
-   Willow Rose Graysen.
    
-   Luna Oliver Graysen.
    
-   Oliver Graysen.
    
-   Olivier Graysen.
    
-   Bailey Graysen.
    
-   Rhu Graysen.
    
-   Kara Luna Graysen.
    

For accounts under one of my legal names or a variation of them, they were fairly easy to recover by submitting identity documents to the email provider and regaining access after 4-6 weeks.

I used a mix of iCloud, Gmail, Fastmail, Protonmail, and Hotmail/Outlook that I needed to recover to get access to old social media accounts and archived blogs.

For accounts that were not referencing one of my legal names, I have slowly been able to recover them by waiting for the time locks to pass, using recovery emails that are other email addresses I’ve recovered.

## Twilio

Some accounts did not have a recovery email address configured and only an old phone number from when I had a local number in British Columbia, Alberta, Quebec, Ontario, New York, or Germany. 

I worked with cellular carriers to reclaim my old phone numbers that hadn’t been reassigned and purchased a line with that phone number, reclaiming it from the pool. To keep costs down, I ported the number to Twilio where I hold them for a $2 per month and can receive SMS codes using some Webhook functions. I may expand on this setup in the future.

Something I found interesting was for one phone number I couldn’t reclaim because there was an active line, I found an old phone bill in my email, texted the number explaining the situation and got consent to send a recovery code and they sent me a screenshot. This was incredibly helpful!

## Consolidating into Gmail

I created a new Gmail account that is separate from my day-to-day use and added it to my family Google One account.

I used the Google Importer tool and imported all emails from each account over POP3 into my new Gmail account. These are tagged by identity and for some accounts with tens of thousands of emails, it took weeks for the import to complete.

Google Gemini has also been super useful to write a prompt like: “Can you show me all emails that contain employment contracts?” and I can quickly have a custom filter to emails containing PDFs of every job I’ve ever had.

## Routing New Email via Fastmail

A lot of these old email accounts only receiving spam mail now, however some accounts still receive important emails but I’m unable to update where the sender will send emails to. For these accounts, I’ve setup email routing through Fastmail.

Each email account has a locked email rule (that can not be removed) that forwards all email to a Fastmail alias.

For example:

`first.last@gmail.com` => `first.last.gmail@fastmail.com`

`last.first@icloud.com` => `last.first.icloud@fastmail.com`

The Fastmail alias then forwards it to my Google account where rules are set up to tag and archive the email or surface it to my inbox if it has markers it’s important.

## Custom Domains

For business emails and various email accounts I previously used domains on Google Workspaces with, I’ve transferred domains to Cloudflare and setup MX records to send wildcard `*@domain.com` to their respective `domain.com@fastmail.com` to the Fastmail routing mechanism.

## Conclusion

All in all, I have:

-   108 email addresses being consolidated into 1 inbox that automatically handles prioritization and archival.
    
-   I have merged Google Drive and Google Photos into a single account, made exports of old YouTube channels and uploaded content into Google Drive.
    
-   I have filters setup to unsubscribe and delete newsletters and spam mail to extend the life of my 2TB of Google Storage.
    
-   I have one email address that I use for outbound email and can reply to any incoming email from my new email address in the same place.
    
-   The desire to not do this again. 😂
    

Thanks for reading!
