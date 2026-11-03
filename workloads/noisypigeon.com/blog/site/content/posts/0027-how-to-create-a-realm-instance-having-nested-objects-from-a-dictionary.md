+++
title = "How to create a Realm instance (having nested objects) from a dictionary?"
date = 2017-02-17T20:00:00+00:00
slug = "how-to-create-a-realm-instance-having-nested-objects-from-a-dictionary"
description = "Mirror of my answer to a Stack Overflow question about Realm nested objects."

[taxonomies]
tags = ["software-engineering"]

[extra]
og_image = "/assets/images/posts/how-to-create-a-realm-instance-having-nested-objects-from-a-dictionary/screenshot-202024-10-06-20at-201-33-39-e2-80-afpm.png"
+++


> 📖 **Note**: This post is from the archives. It may reflect outdated views or a [previous name](/pages/names/). This post is a mirror of my answer to a Stack Overflow question from February 17th, 2017 as Willow PXXX.
> 
> \[[Original (Question)](https://stackoverflow.com/a/42308579)\], \[[Original (Answer)](https://stackoverflow.com/a/42308579)\]

<figure class="attachment attachment--preview attachment--png"><img height="114" width="1050" alt="An image with caption: Comment after providing the answer below. [&lt;a target=&quot;_blank&quot; rel=&quot;noopener noreferrer nofollow&quot; href=&quot;https://stackoverflow.com/questions/42294962/how-to-create-a-realm-instance-having-nested-objects-from-a-dictionary/42308579#comment71842464_42308579&quot;&gt;Source&lt;/a&gt;]" loading="lazy" src="/assets/images/posts/how-to-create-a-realm-instance-having-nested-objects-from-a-dictionary/screenshot-202024-10-06-20at-201-33-39-e2-80-afpm.png"><figcaption class="attachment__caption" aria-hidden="true">Comment after providing the answer below. [<a href="https://stackoverflow.com/questions/42294962/how-to-create-a-realm-instance-having-nested-objects-from-a-dictionary/42308579#comment71842464_42308579">Source</a>]</figcaption></figure>

## My Answer

> The reason you're experiencing these issues is because the `List<User>` object must be initialized before use. Realm recommends declaring `List` and `RealmOptional` objects as `let` and therefore initializing them right inside the model.
> 
> In addition to that, you need to declare other properties using the `dynamic var` attribute in order for the properties to become accessors for the underlying database.
> 
> More on what I mentioned in the [Property Attributes](https://realm.io/docs/swift/latest/#property-attributes) and [Supported Types](https://realm.io/docs/swift/latest/#supported-types) sections of our documentation. With all that said, I recommend you change your model to the following:
> 
> ```
> import RealmSwift
> 
> class User: Object {
>     dynamic var firstName: String?
>     dynamic var lastName: String?
>     let friends = List<User>()
> }
> ```
> 
> I hope this helps and please let me know if you need anymore assistance.

## Question

Asked by “Mohshin Shah”:

> For example, I have model `class` like this
> 
> ```
> class User: Objects {
>     var firstName: String?
>     var lastName: String?
>     var friends: List<User>?
> }
> ```
> 
> and `Dictionary` like this
> 
> ```
> {
>    firstName : "Mohshin"
>    lastName : "Shah"
>    friends : [
>               {
>                firstName : "Friend 1"
>                lastName : "abc"
>                },
>               {
>                firstName : "Friend 2"
>                lastName : "def"
>                }
>               ]
> }
> ```
> 
> I want to make a User object using the dictionary. I have tried Realm' `init(value: Any)` method and it is creating the User with flat values only but not the friends is initiated.
