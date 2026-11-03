+++
title = "How to add custom colours to UIColor()"
date = 2015-04-22T19:00:00+00:00
slug = "how-to-add-custom-colours-to-uicolor"
description = "Swift UIColor convenience how-to."

[taxonomies]
tags = ["software-engineering"]
+++


I have noticed that a lot of iOS (and Mac) developers who need to use custom colours in their applications formulate their palette using:

```
let fooColor = UIColor(red: X, green: X, blue: X, alpha: X)
```

As much as this solution works, it creates unnecessary clutter if you’re using this constant in multiple scopes. I use a more efficient solution that allows for less clutter and reduces the weight of the application.

#### 1\. Create a new Swift class.

Please create a new Swift file (cmd+n => Swift file) with the name “CustomColorPalette”.

#### 2\. Add UIColor extension.

Now that you have your new Swift file, open it up in your Xcode editor. We will need to import _UIKit_, in order to extend the _UIColor_ class.

```
import UIKit
```

With _UIKit_ imported, extend the _UIColor_ class using the following:

```
extension UIColor {}
```

#### 3\. Add custom colours.

Each custom colour can be added to your extension using the following declaration inside of your _UIColor_ extension.

```
class func customColor() -> UIColor {
    return UIColor(red: X, green: X, blue: X, alpha: X)
}
```

This method is using the _class_ keyword, so the custom colour can be fetched with the following line:

```
view.backgroundColor = UIColor.customColor()
```
